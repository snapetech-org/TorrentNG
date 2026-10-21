// Includes the unmodified production rate scheduler from commit db9e65a.
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

fn main() {}
