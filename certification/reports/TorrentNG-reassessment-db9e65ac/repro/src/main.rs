use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;
use rt_storage::{IoClass, MountScheduler, SchedulerConfig};
use rt_path::{StorageProfile, StorageRootId};

static COUNT: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
struct Counter;
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p=System.alloc(l);
        if COUNT.load(Relaxed) && !p.is_null() { ALLOCS.fetch_add(1,Relaxed); ALLOC_BYTES.fetch_add(l.size() as u64,Relaxed); }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p=System.alloc_zeroed(l);
        if COUNT.load(Relaxed) && !p.is_null() { ALLOCS.fetch_add(1,Relaxed); ALLOC_BYTES.fetch_add(l.size() as u64,Relaxed); }
        p
    }
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 {
        let q=System.realloc(p,l,n);
        if COUNT.load(Relaxed) && !q.is_null() { ALLOCS.fetch_add(1,Relaxed); ALLOC_BYTES.fetch_add(n as u64,Relaxed); }
        q
    }
    unsafe fn dealloc(&self,p:*mut u8,l:Layout) { System.dealloc(p,l); }
}
#[global_allocator] static ALLOCATOR: Counter = Counter;

fn cpu_seconds()->f64 {
    let mut ts=libc::timespec{tv_sec:0,tv_nsec:0};
    assert_eq!(unsafe{libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID,&mut ts)},0);
    ts.tv_sec as f64+ts.tv_nsec as f64*1e-9
}
fn main() {
    let args:Vec<_>=std::env::args().collect();
    let mode=args.get(1).map(String::as_str).unwrap_or("cache");
    let block:usize=args.get(2).map(|s|s.parse().unwrap()).unwrap_or(16384);
    let total:usize=args.get(3).map(|s|s.parse().unwrap()).unwrap_or(64*1024*1024);
    std::env::set_var("TNG_STORAGE_BACKEND","pread");
    let rt=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(async {
        let path=std::env::temp_dir().join(format!("tng-audit-probe-{}.bin",std::process::id()));
        let file_len=16*1024*1024;
        std::fs::write(&path,vec![0xa5u8;file_len]).unwrap();
        let mut config=SchedulerConfig { profile:StorageProfile::Nvme, resources:Some(rt_metrics::ResourceGovernor::new(Default::default())), ..Default::default() };
        if mode=="no_cache" { config.storage_io.peer_read_cache_bytes=0; config.storage_io.peer_read_cache_entries=0; }
        if mode=="small_cache" {config.storage_io.peer_read_cache_bytes=512*1024;}
        if mode=="pool_aligned" {config.storage_io.peer_read_readahead_bytes=256*1024;}
        // No deadline-based elevator wait is part of this probe.
        config.storage_io.peer_read_elevator_budget_ms=0;
        let scheduler=MountScheduler::new(StorageRootId::new(),&config);
        for i in 0..128 { let data=scheduler.read_at(IoClass::PeerRead,&path,((i*block)%file_len) as u64,block).await.unwrap(); assert!(data.iter().all(|b|*b==0xa5)); }
        let before=scheduler.stats();
        ALLOCS.store(0,Relaxed); ALLOC_BYTES.store(0,Relaxed);
        let start_cpu=cpu_seconds();let start=Instant::now();COUNT.store(true,Relaxed);
        for i in 0..total/block {
            let data=scheduler.read_at(IoClass::PeerRead,&path,((i*block)%file_len) as u64,block).await.unwrap();
            assert_eq!(data.len(),block);assert_eq!(data[0],0xa5);assert_eq!(data[block-1],0xa5);
            std::hint::black_box(&data);
        }
        COUNT.store(false,Relaxed);let cpu=cpu_seconds()-start_cpu;let wall=start.elapsed().as_secs_f64();
        let allocs=ALLOCS.load(Relaxed);let alloc_bytes=ALLOC_BYTES.load(Relaxed);
        let after=scheduler.stats();let pool=rt_storage::global_frame_pool();
        println!("{{\"mode\":\"{}\",\"block\":{},\"payload_bytes\":{},\"cpu_s_per_gib\":{},\"wall_s\":{},\"allocations\":{},\"allocated_bytes\":{},\"allocations_per_read\":{},\"allocated_bytes_per_read\":{},\"cache_hits\":{},\"cache_misses\":{},\"cache_bytes\":{},\"cache_backing_bytes\":{},\"pool_live\":{},\"pool_idle\":{}}}",mode,block,total,cpu/(total as f64/(1u64<<30) as f64),wall,allocs,alloc_bytes,allocs as f64/(total/block) as f64,alloc_bytes as f64/(total/block) as f64,after.peer_read_cache_hits-before.peer_read_cache_hits,after.peer_read_cache_misses-before.peer_read_cache_misses,after.peer_read_cache_bytes,after.peer_read_cache_global_backing_bytes,pool.in_use_bytes(),pool.retained_bytes());
        drop(scheduler);
        println!("{{\"after_scheduler_drop_pool_live\":{},\"after_scheduler_drop_pool_idle\":{}}}",pool.in_use_bytes(),pool.retained_bytes());
        std::fs::remove_file(path).unwrap();
    });
}
