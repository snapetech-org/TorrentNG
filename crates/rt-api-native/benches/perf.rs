use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rt_api_native::AppState;
use rt_session::{SessionRegistry, TorrentEntry, TorrentState};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::RwLock;

const TORRENT_COUNTS: [usize; 3] = [1_000, 10_000, 100_000];
const CACHED_READS: usize = 1_000;
const CHANGED_SAMPLES: usize = 3;
const SNAPSHOT_MAX_AGE_PLUS_MARGIN: Duration = Duration::from_millis(760);

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_allocation(bytes: usize) {
    if TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation(new_size);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

fn reset_allocation_counts() {
    TRACK_ALLOCATIONS.store(false, Ordering::Relaxed);
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
}

fn allocation_counts() -> (u64, u64) {
    (
        ALLOCATION_CALLS.load(Ordering::Relaxed),
        ALLOCATED_BYTES.load(Ordering::Relaxed),
    )
}

fn torrent_hash(index: usize) -> String {
    format!("{index:040x}")
}

fn build_state(runtime: &Runtime, torrent_count: usize) -> AppState {
    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    {
        let mut registry = runtime.block_on(registry.write());
        for index in 0..torrent_count {
            let mut entry = TorrentEntry::new(
                torrent_hash(index + 1),
                format!("fixture-{index:08}"),
                "/task-scratch/perf-111".to_owned(),
            );
            entry.total_length = 1024 * 1024;
            entry.amount_left = 1024 * 1024;
            entry.state = TorrentState::Paused;
            registry.add(entry).expect("fixture infohashes are unique");
        }
    }
    AppState::with_registry(registry)
}

fn emit_sample(
    runtime: &Runtime,
    state: &AppState,
    probe: &str,
    torrent_count: usize,
    samples: usize,
) {
    let (total_ns, allocation_calls, allocated_bytes) = runtime.block_on(async {
        reset_allocation_counts();
        TRACK_ALLOCATIONS.store(true, Ordering::Relaxed);
        let started = Instant::now();
        for _ in 0..samples {
            let snapshot = state
                .torrent_snapshot(None)
                .await
                .expect("snapshot refresh succeeds");
            black_box((snapshot.revision, snapshot.torrents.len()));
        }
        let total_ns = started.elapsed().as_nanos();
        TRACK_ALLOCATIONS.store(false, Ordering::Relaxed);
        let (allocation_calls, allocated_bytes) = allocation_counts();
        (total_ns, allocation_calls, allocated_bytes)
    });
    let ns_per_sample = total_ns / samples as u128;
    println!(
        "{probe},{torrent_count},{samples},{total_ns},{ns_per_sample},{allocation_calls},{allocated_bytes}"
    );
}

fn update_one_torrent(runtime: &Runtime, state: &AppState, torrent_count: usize, sample: usize) {
    let info_hash = torrent_hash(torrent_count / 2 + 1);
    runtime.block_on(async {
        let mut registry = state.registry.write().await;
        let mut entry = registry
            .get_mut(&info_hash)
            .expect("changed fixture torrent exists");
        entry
            .name
            .push(if sample.is_multiple_of(2) { 'x' } else { 'y' });
    });
}

fn run_count(runtime: &Runtime, torrent_count: usize) {
    let state = build_state(runtime, torrent_count);

    emit_sample(runtime, &state, "snapshot_initial", torrent_count, 1);
    // Warm the current-thread executor outside the allocation window, then
    // measure reads of the identical cached snapshot as a separate baseline.
    let _ = runtime
        .block_on(state.torrent_snapshot(None))
        .expect("cached snapshot is available");
    emit_sample(
        runtime,
        &state,
        "snapshot_cached_unchanged",
        torrent_count,
        CACHED_READS,
    );

    for sample in 0..CHANGED_SAMPLES {
        update_one_torrent(runtime, &state, torrent_count, sample);
        // AppState intentionally serves its current-generation snapshot for
        // up to 750 ms. Wait outside measurement to exercise a real refresh.
        runtime.block_on(async {
            tokio::time::sleep(SNAPSHOT_MAX_AGE_PLUS_MARGIN).await;
        });
        emit_sample(
            runtime,
            &state,
            "snapshot_one_row_changed",
            torrent_count,
            1,
        );
    }
}

fn main() {
    println!("probe,torrent_count,samples,total_ns,ns_per_sample,allocation_calls,allocated_bytes");
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread benchmark runtime builds");
    for torrent_count in TORRENT_COUNTS {
        run_count(&runtime, torrent_count);
    }
}
