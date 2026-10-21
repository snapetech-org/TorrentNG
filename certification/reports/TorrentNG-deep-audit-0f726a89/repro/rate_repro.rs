// Includes the unmodified production rate scheduler from commit 0f726a89.
// Only its identity types are reproduced here to avoid unrelated crate dependencies.
#![allow(dead_code)]
type Nanos = u64;
mod coords {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct ConnectionId { pub slot: u32, pub generation: u32 }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct TorrentKey(pub u64);
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct TransferPeerId { pub torrent: TorrentKey, pub connection: ConnectionId }
    impl TransferPeerId {
        pub const fn new(torrent: TorrentKey, connection: ConnectionId) -> Self {
            Self { torrent, connection }
        }
    }
}
#[path = "../source/crates/rt-engine/src/transfer/rate.rs"]
mod rate;
use coords::{ConnectionId, TorrentKey, TransferPeerId};
use rate::{Grant, RateTree};
const BLOCK: u64 = 16 * 1024;
fn peer(slot: u32) -> TransferPeerId {
    TransferPeerId::new(TorrentKey(1), ConnectionId { slot, generation: 1 })
}
fn setup() -> RateTree {
    let mut tree = RateTree::new(Some(32 * 1024 * 1024), 0);
    tree.add_torrent(TorrentKey(1), Some(32 * 1024 * 1024), 0);
    tree.add_peer(peer(1), 1);
    tree.add_peer(peer(2), 1);
    tree
}
fn spend(tree: &mut RateTree, p: TransferPeerId, now: u64) -> Grant {
    let (result, id) = tree.try_grant_tracked(p, BLOCK, now);
    if let Some(id) = id { assert_eq!(tree.settle_grant(p, id, BLOCK), BLOCK); }
    result
}
// Reproduces the live adapter lifecycle: a denied peer later becomes choked
// or cancels its wait, without disconnecting/removing its registration.
fn parked_peer_case(remove_parked: bool) -> (usize, usize, u64) {
    let mut tree = setup();
    while spend(&mut tree, peer(1), 0) == Grant::Granted {}
    assert!(matches!(spend(&mut tree, peer(2), 0), Grant::Wait { .. }));
    if remove_parked { tree.remove_peer(peer(2)); }
    let mut granted = 0;
    let mut denied = 0;
    let mut last_delay = 0;
    for i in 0..1000 {
        let now = 1_000_000_000 + i * 1_000_000;
        match spend(&mut tree, peer(1), now) {
            Grant::Granted => granted += 1,
            Grant::Wait { until } => { denied += 1; last_delay = until - now; }
            Grant::Unknown => panic!("lost active peer"),
        }
    }
    (granted, denied, last_delay)
}
fn main() {
    let parked = parked_peer_case(false);
    let removed = parked_peer_case(true);
    println!("denied peer remains connected but stops retrying: grants={} denials={} last_retry_delay_ns={}", parked.0, parked.1, parked.2);
    println!("same schedule, remove that peer before retrying: grants={} denials={}", removed.0, removed.1);
    assert_eq!(parked.0, 0);
    assert_eq!(parked.1, 1000);
    assert_eq!(parked.2, 1);
    assert_eq!(removed.0, 1000);
    println!("CONFIRMED: stale demand can block a funded active peer; a 1 ns retry does not repair it.");
    // Isolate uncontended single-peer scheduling cost. This does not include
    // Tokio, storage, socket I/O or multi-threaded lock contention.
    for limited in [true, false] {
        let limit=if limited {Some(32*1024*1024)} else {None};
        let mut tree=RateTree::new(limit,0);
        tree.add_torrent(TorrentKey(1),limit,0); tree.add_peer(peer(1),1);
        let tree=std::sync::Mutex::new(tree);
        const OPS:u64=1_048_576;
        let start=std::time::Instant::now();
        for i in 0..OPS {
            let now=i*500_000;
            let (grant,id)=tree.lock().unwrap().try_grant_tracked(peer(1),BLOCK,now);
            assert_eq!(grant,Grant::Granted); let id=id.unwrap();
            tree.lock().unwrap().settle_grant(peer(1),id,BLOCK);
            tree.lock().unwrap().refund_grant(peer(1),id,0,now);
        }
        let elapsed=start.elapsed().as_secs_f64();
        println!("uncontended rate core limited={limited}: operations={OPS} wall_seconds_per_GiB={:.6}", elapsed/16.);
    }
}
