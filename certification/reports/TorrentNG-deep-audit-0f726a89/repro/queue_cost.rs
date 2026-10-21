// Mechanism benchmark, NOT a TorrentNG or competitor benchmark.
// Same warm file and useful byte count; compare one versus two worker round trips.
// Uses std channels, not Tokio. Absolute numbers do not predict application gains.
use std::{fs::{File, OpenOptions}, hint::black_box, os::unix::fs::FileExt,
    sync::mpsc::{sync_channel, SyncSender, Receiver}, thread, time::Instant};
#[repr(C)] struct Timespec { sec: i64, nsec: i64 }
unsafe extern "C" { fn clock_gettime(clock: i32, out: *mut Timespec) -> i32; }
fn cpu() -> f64 { let mut t=Timespec{sec:0,nsec:0}; assert_eq!(unsafe{clock_gettime(2,&mut t)},0); t.sec as f64+t.nsec as f64/1e9 }
fn job_worker(path: String, bytes: usize, kind: u8) -> (SyncSender<u64>, Receiver<u8>, thread::JoinHandle<()>) {
    let (tx,rx)=sync_channel::<u64>(8); let (done_tx,done)=sync_channel(8);
    let join=thread::spawn(move || {
        let f=File::open(&path).unwrap(); let mut buf=vec![0u8;bytes];
        while let Ok(off)=rx.recv() {
            if kind!=1 { black_box(std::fs::symlink_metadata(&path).unwrap()); }
            if kind!=0 { f.read_exact_at(&mut buf,off).unwrap(); }
            if kind==2 { black_box(std::fs::symlink_metadata(&path).unwrap()); }
            done_tx.send(black_box(buf[0])).unwrap();
        }
    }); (tx,done,join)
}
fn run(path:&str, mode:u8, block:usize) -> (f64,f64) {
    const TOTAL:usize=256*1024*1024;
    let n=TOTAL/block;
    let f=File::open(path).unwrap(); let mut buf=vec![0u8;block];
    let prep=if mode==2 {Some(job_worker(path.into(),block,0))}else{None};
    let io=if mode>0 {Some(job_worker(path.into(),block,if mode==1{2}else{1}))}else{None};
    let start_cpu=cpu(); let start=Instant::now();
    for i in 0..n {
        let off=((i*block)%(16*1024*1024)) as u64;
        if mode==0 {
            black_box(std::fs::symlink_metadata(path).unwrap());
            f.read_exact_at(&mut buf,off).unwrap();
            black_box(std::fs::symlink_metadata(path).unwrap());
            black_box(buf[0]);
        } else {
            if let Some((tx,rx,_))=&prep { tx.send(off).unwrap(); black_box(rx.recv().unwrap()); }
            let (tx,rx,_)=io.as_ref().unwrap(); tx.send(off).unwrap(); black_box(rx.recv().unwrap());
            if mode==2 { black_box(std::fs::symlink_metadata(path).unwrap()); }
        }
    }
    let result=((cpu()-start_cpu)*4., start.elapsed().as_secs_f64()*4.);
    if let Some((tx,rx,j))=prep {drop(tx);drop(rx);j.join().unwrap();}
    if let Some((tx,rx,j))=io {drop(tx);drop(rx);j.join().unwrap();}
    result
}
fn main(){
    let path=std::env::temp_dir().join(format!("tng-queue-probe-{}",std::process::id()));
    let f=OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    f.write_all_at(&vec![7u8;16*1024*1024],0).unwrap(); drop(f);
    println!("mechanism,repeat,block_bytes,cpu_seconds_per_GiB,wall_seconds_per_GiB");
    let modes=[("inline_warm_read",0,16384),("one_worker_round_trip",1,16384),("two_worker_round_trips",2,16384),("one_worker_batched_256KiB",1,262144)];
    for repeat in 0..3 { for j in 0..4 {let (name,mode,block)=modes[(j+repeat)%4]; let (cpu,wall)=run(path.to_str().unwrap(),mode,block); println!("{name},{repeat},{block},{cpu:.6},{wall:.6}");}}
    std::fs::remove_file(path).unwrap();
}
