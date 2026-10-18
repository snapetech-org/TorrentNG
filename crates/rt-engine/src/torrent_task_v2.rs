//! Pure BEP 52 torrent task.
//!
//! The original torrent actor is intentionally v1-shaped: its picker and
//! storage map operate on SHA-1 pieces in the concatenated v1 file stream.
//! Pure v2 torrents have a different piece address space (each file starts on
//! a piece boundary) and a SHA-256 merkle verification rule, so keeping this
//! actor separate prevents a v2 compatibility path from silently changing v1
//! semantics.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use rt_hash::{merkle_layers, BlockHash, V2_BLOCK_SIZE};
#[cfg(test)]
use rt_hash::{merkle_root, piece_layer_hash};
use rt_metainfo::{TorrentFileV2, TorrentMetaV2};
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};
use rt_path::{StorageProfile, StorageRootId};
use rt_peer_manager::{
    ChokeDecision, ChokeMode, ChokeState, Choker, PeerId, PeerSnapshot, DEFAULT_MAX_UNCHOKED,
};
use rt_peer_wire::{
    codec::PeerCodec,
    extension::{ExtensionHandshake, EXT_HANDSHAKE_ID},
    handshake::{ExtensionFlags, Handshake},
    message::Message,
};
use rt_piece_map::{V2FileSpan, V2PieceMap};
use rt_session::{SessionRegistry, TorrentEntry, TorrentState};
use rt_storage::{
    scheduler::{scheduled_read_owned, scheduled_write, scheduled_write_with_shared_payload_lease},
    IoClass, MountScheduler, SchedulerConfig, StorageIoConfig, V2FileHash, V2FileVerifier,
    VerifyResult,
};
use rt_tracker::{TrackerEvent, TrackerState, TrackerStatus};
use rt_utp::UtpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, RwLock};
use tokio::time::{interval, sleep, timeout};
use tokio_util::codec::Framed;
use tracing::{debug, warn};

use crate::crash_safety::CrashSafetyRuntime;
use crate::db_worker::DbExecutor;
use crate::egress_policy::OutboundEgressPolicy;
use crate::network_budget::{GlobalNetworkBudget, RateLimitCancellation};
use crate::torrent_task::{
    outgoing_transport_policy_configured, outgoing_transport_policy_for_peer,
    prepare_tracker_peer_cache, private_peer_source_allowed, restore_tracker_state_from_rows,
    tracker_peer_cache_cap, tracker_tiers_from_urls, OutgoingTransportPolicy, PeerIo, PeerSource,
    TorrentCmd, UtpPeerIo,
};
use crate::tracker_runtime::{
    announce_tracker, first_tracker_key, next_tracker_key, protocol_numwant,
    tracker_keys_for_announce, tracker_keys_for_stopped_announce, url_log_target,
    TrackerAnnounceContext, TrackerAnnounceResult, TrackerAnnounceSpec, TrackerKey, TrackerWorkers,
    MAX_TRACKER_ANNOUNCES_IN_FLIGHT, STOPPED_TRACKER_ANNOUNCE_DEADLINE,
};
use crate::transfer::coords::{BlockExtent, PieceGeometry};
use crate::{EnginePeerSnapshot, EngineTorrentLimits, EngineWebseedSnapshot, TorrentRuntimeStats};

const V2_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const V2_SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const V2_PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const V2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);
const V2_CHOKE_INTERVAL: Duration = Duration::from_secs(10);
const V2_REQUEST_WINDOW: usize = 32;
const V2_MAX_PENDING_UPLOAD_READS: usize = 16;
const V2_MAX_QUEUED_UPLOAD_REQUESTS: usize = 128;
const V2_MAX_UPLOAD_WORK: usize = V2_MAX_PENDING_UPLOAD_READS + V2_MAX_QUEUED_UPLOAD_REQUESTS;
const V2_MAX_PENDING_PIECE_JOBS: usize = 16;
const V2_MAX_PENDING_TRANSFER_JOBS: usize = V2_REQUEST_WINDOW;
const V2_MAX_TRACKED_PIECES_PER_PEER: usize = V2_REQUEST_WINDOW;
// `TorrentMetaV2` accepts every piece length representable by the peer-wire
// u32 coordinates. The old 65,536-block guard silently rejected valid pieces
// above about 1 GiB. Keep a protocol-derived bound instead; the assembly byte
// cap and ResourceGovernor still bound the actual retained content.
const V2_MAX_PIECE_BLOCKS: usize =
    (u32::MAX as usize).div_ceil(rt_peer_wire::message::MAX_BLOCK_SIZE as usize);
/// Leaf hashes are computed from reads of this size, so a hash request never
/// holds a file's payload in memory and large files are not rejected for size.
const V2_HASH_EXCHANGE_READ_BYTES: usize = 256 * 1024;
/// Bytes of authenticated Merkle layers kept for hash-request serving, shared
/// by every peer of one torrent (about 0.4% of the covered file bytes).
const V2_PROOF_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const V2_MAX_HASH_REQUEST_LENGTH: u32 = 512;
const STOPPED_TRACKER_CONTROL_ANNOUNCE_DEADLINE: Duration = Duration::from_secs(1);

/// Preserve BEP 12's tier failover semantics for pure-v2 metainfo. The flat
/// tracker list is still used for durable client overrides, but an untouched
/// torrent must keep the announce-list's tier boundaries.
fn tracker_tiers_from_meta_v2(meta: &TorrentMetaV2) -> Vec<Vec<TrackerState>> {
    let mut seen = HashSet::new();
    let mut tiers = Vec::new();

    for tier in &meta.announce_list {
        let trackers = tier
            .iter()
            .filter_map(|url| {
                if url.is_empty() || !seen.insert(url.clone()) {
                    None
                } else {
                    Some(TrackerState::new(url.clone()))
                }
            })
            .collect::<Vec<_>>();
        if !trackers.is_empty() {
            tiers.push(trackers);
        }
    }

    if let Some(url) = &meta.announce {
        if !url.is_empty() && seen.insert(url.clone()) {
            tiers.insert(0, vec![TrackerState::new(url.clone())]);
        }
    }
    tiers
}

/// Estimate one packed v2 bitmap allocation.
pub(crate) fn v2_bitmap_memory_bytes(piece_count: usize) -> usize {
    piece_count
        .div_ceil(64)
        .saturating_mul(std::mem::size_of::<u64>())
}

/// Estimate the two torrent-wide v2 availability bitmaps that can overlap
/// during a recheck or copy-on-write update: the current map and the map
/// visible to peers while the replacement is built.
pub(crate) fn v2_piece_index_memory_bytes(piece_count: usize) -> usize {
    v2_bitmap_memory_bytes(piece_count).saturating_mul(2)
}

/// Estimate the immutable v2 graph retained by a live task. The exact parser
/// allocation is deliberately charged before the actor is constructed, just
/// as it is for the v1 actor; this covers paths, file descriptors' metadata,
/// piece-layer vectors, and the retained raw info dictionary. The actor also
/// keeps an original-file clone for policy rebuilds and path-bearing spans in
/// V2PieceMap, so include those two additional file graphs.
fn v2_file_graph_memory_bytes(files: &[TorrentFileV2], capacity: usize) -> usize {
    let mut total = capacity.saturating_mul(std::mem::size_of::<TorrentFileV2>());
    for file in files {
        total = total.saturating_add(
            file.path
                .components()
                .len()
                .saturating_mul(std::mem::size_of::<String>()),
        );
        for component in file.path.components() {
            total = total.saturating_add(component.capacity());
        }
    }
    total
}

pub(crate) fn persistent_torrent_metadata_memory_bytes_v2(meta: &TorrentMetaV2) -> usize {
    let mut total = 64 * 1024usize;
    total = total.saturating_add(meta.name.capacity());
    total = total.saturating_add(meta.raw.capacity());
    total = total.saturating_add(meta.announce.as_ref().map_or(0, String::capacity));
    total = total.saturating_add(meta.comment.as_ref().map_or(0, String::capacity));
    total = total.saturating_add(meta.created_by.as_ref().map_or(0, String::capacity));
    total = total.saturating_add(
        meta.announce_list
            .capacity()
            .saturating_mul(std::mem::size_of::<Vec<String>>()),
    );
    for tier in &meta.announce_list {
        total = total.saturating_add(
            tier.capacity()
                .saturating_mul(std::mem::size_of::<String>()),
        );
        for tracker in tier {
            total = total.saturating_add(tracker.capacity());
        }
    }
    total = total.saturating_add(
        meta.webseeds
            .capacity()
            .saturating_mul(std::mem::size_of::<String>()),
    );
    for webseed in &meta.webseeds {
        total = total.saturating_add(webseed.capacity());
    }
    total = total.saturating_add(
        v2_file_graph_memory_bytes(&meta.files, meta.files.capacity()).saturating_mul(3),
    );
    total = total.saturating_add(
        meta.piece_layers
            .capacity()
            .saturating_mul(std::mem::size_of::<([u8; 32], Vec<[u8; 32]>)>()),
    );
    for (root, hashes) in &meta.piece_layers {
        let _ = root;
        total = total.saturating_add(
            hashes
                .capacity()
                .saturating_mul(std::mem::size_of::<[u8; 32]>()),
        );
    }
    total
}

const MAX_V2_REGRESSION_PIECES: usize = 32;
const MAX_V2_REGRESSION_FILES: usize = 20;

/// Pick the files a v2 `sample` read-back verifies: a deterministic
/// pseudo-random order, taken until at least `percent` of the bytes are
/// covered, and always at least one file. Returns `None` when that would be
/// every file anyway (a full verification costs no more).
fn select_v2_sample_files(files: &[(u32, u64)], percent: u8, seed: u64) -> Option<Vec<u32>> {
    if files.is_empty() || percent >= 100 {
        return None;
    }
    let total: u64 = files.iter().map(|(_, length)| *length).sum();
    let budget = (u128::from(total) * u128::from(percent)).div_ceil(100) as u64;
    let mut order: Vec<(u64, u32, u64)> = files
        .iter()
        .map(|(index, length)| {
            let mut state = seed ^ u64::from(*index).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            (crate::torrent_task::splitmix64(&mut state), *index, *length)
        })
        .collect();
    order.sort_unstable();
    let mut picked = Vec::new();
    let mut covered = 0u64;
    for (_, index, length) in order {
        if !picked.is_empty() && covered >= budget {
            break;
        }
        picked.push(index);
        covered = covered.saturating_add(length);
    }
    if picked.len() >= files.len() {
        return None;
    }
    picked.sort_unstable();
    Some(picked)
}

#[derive(Debug, Clone)]
struct V2Bitmap {
    len: usize,
    words: Vec<u64>,
}

impl V2Bitmap {
    fn memory_bytes_for_len(len: usize) -> u64 {
        v2_bitmap_memory_bytes(len) as u64
    }

    fn new(len: usize) -> Self {
        Self {
            len,
            words: vec![0; len.div_ceil(64)],
        }
    }

    fn copy_from_bitfield(&mut self, bits: &[u8]) -> Result<(), String> {
        let piece_count = self.len;
        if bits.len() != piece_count.div_ceil(8) {
            return Err(format!(
                "v2 bitfield has {} bytes, expected {}",
                bits.len(),
                piece_count.div_ceil(8)
            ));
        }
        if let Some(remainder) = piece_count.checked_rem(8).filter(|value| *value != 0) {
            let unused_mask = (1_u8 << (8 - remainder)) - 1;
            if bits.last().is_some_and(|byte| byte & unused_mask != 0) {
                return Err("v2 bitfield sets bits beyond the piece count".to_owned());
            }
        }
        self.fill(false);
        for (byte_index, byte) in bits.iter().copied().enumerate() {
            for bit in 0..8 {
                let piece = byte_index * 8 + bit;
                if piece >= piece_count {
                    break;
                }
                if byte & (0x80 >> bit) != 0 {
                    self.set(piece, true);
                }
            }
        }
        Ok(())
    }

    fn fill(&mut self, value: bool) {
        self.words.fill(if value { u64::MAX } else { 0 });
        if value {
            if let Some(last) = self.words.last_mut() {
                if !self.len.is_multiple_of(64) {
                    *last = (1_u64 << (self.len % 64)) - 1;
                }
            }
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, index: usize) -> bool {
        index < self.len && self.words[index / 64] & (1_u64 << (index % 64)) != 0
    }

    fn set(&mut self, index: usize, value: bool) {
        if index >= self.len {
            return;
        }
        let word = &mut self.words[index / 64];
        let mask = 1_u64 << (index % 64);
        if value {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }

    fn count_ones(&self) -> usize {
        self.words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index + 1 == self.words.len() && !self.len.is_multiple_of(64) {
                    word & ((1_u64 << (self.len % 64)) - 1)
                } else {
                    *word
                }
            })
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    fn to_bitfield(&self) -> Vec<u8> {
        let mut bits = vec![0_u8; self.len.div_ceil(8)];
        for index in 0..self.len {
            if self.get(index) {
                bits[index / 8] |= 0x80 >> (index % 8);
            }
        }
        bits
    }
}

#[derive(Debug)]
struct V2PeerState {
    remote_have: Arc<V2Bitmap>,
    choked: bool,
    upload_choked: bool,
    interested: bool,
    downloaded: u64,
    uploaded: u64,
    download_rate: i64,
    upload_rate: i64,
    download_window: u64,
    upload_window: u64,
    rate_window_started: Instant,
    outstanding: usize,
    assembly_buffers: u64,
    assembly_bytes: u64,
}

impl V2PeerState {
    fn new(piece_count: usize) -> Self {
        Self {
            remote_have: Arc::new(V2Bitmap::new(piece_count)),
            choked: true,
            upload_choked: true,
            interested: false,
            downloaded: 0,
            uploaded: 0,
            download_rate: 0,
            upload_rate: 0,
            download_window: 0,
            upload_window: 0,
            rate_window_started: Instant::now(),
            outstanding: 0,
            assembly_buffers: 0,
            assembly_bytes: 0,
        }
    }

    fn record_transfer(&mut self, upload: bool, bytes: u64) {
        let now = Instant::now();
        if upload {
            self.uploaded = self.uploaded.saturating_add(bytes);
            self.upload_window = self.upload_window.saturating_add(bytes);
        } else {
            self.downloaded = self.downloaded.saturating_add(bytes);
            self.download_window = self.download_window.saturating_add(bytes);
        }
        let elapsed = now.saturating_duration_since(self.rate_window_started);
        if elapsed >= Duration::from_secs(1) {
            let seconds = elapsed.as_secs_f64().max(0.001);
            self.download_rate = (self.download_window as f64 / seconds).round() as i64;
            self.upload_rate = (self.upload_window as f64 / seconds).round() as i64;
            self.download_window = 0;
            self.upload_window = 0;
            self.rate_window_started = now;
        }
    }

    fn rate(&self, upload: bool) -> i64 {
        if Instant::now().saturating_duration_since(self.rate_window_started)
            > Duration::from_secs(15)
        {
            0
        } else if upload {
            self.upload_rate
        } else {
            self.download_rate
        }
    }
}

struct V2PeerHandle {
    state: Arc<Mutex<V2PeerState>>,
    choker_id: PeerId,
    peer_control: RateLimitCancellation,
    task: tokio::task::JoinHandle<()>,
    _bitmap_memory_lease: MemoryLease,
}

#[derive(Debug)]
enum V2PeerEvent {
    Downloaded { peer: SocketAddr, bytes: u64 },
    Uploaded { peer: SocketAddr, bytes: u64 },
    InterestChanged,
    Disconnected { peer: SocketAddr },
}

#[derive(Clone)]
struct V2PeerContext {
    info_hash: [u8; 20],
    peer: SocketAddr,
    choker_id: PeerId,
    meta: Arc<TorrentMetaV2>,
    piece_map: Arc<V2PieceMap>,
    save_root: PathBuf,
    storage: MountScheduler,
    resources: ResourceGovernor,
    network_budget: GlobalNetworkBudget,
    peer_control: RateLimitCancellation,
    peer_generation: u64,
    upload_control: RateLimitCancellation,
    local_have: Arc<RwLock<Arc<V2Bitmap>>>,
    have_updates: watch::Sender<()>,
    upload_unchoked: watch::Receiver<Arc<HashSet<PeerId>>>,
    file_policy: Arc<HashMap<u32, (bool, i64)>>,
    assembly_cap_bytes: usize,
    events: mpsc::Sender<V2PeerEvent>,
    proof_cache: Arc<V2ProofCache>,
}

/// Merkle layers of a file, proven against its metainfo `pieces root`.
type V2ProofLayers = Arc<Vec<Vec<[u8; 32]>>>;

/// Bounded LRU of proven Merkle layers keyed by `pieces root`.
///
/// A hash request used to reread the whole file and rebuild its tree. The
/// layers are derived only from content that hashes to the metainfo root, so
/// they are authenticated hashes of the torrent rather than of whatever is on
/// disk, and are safe to keep and share across peers.
#[derive(Default)]
struct V2ProofCache {
    inner: Mutex<V2ProofCacheInner>,
}

#[derive(Default)]
struct V2ProofCacheInner {
    entries: HashMap<[u8; 32], (V2ProofLayers, u64, u64)>,
    bytes: u64,
    tick: u64,
}

impl V2ProofCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, V2ProofCacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn get(&self, root: &[u8; 32]) -> Option<V2ProofLayers> {
        let mut inner = self.lock();
        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;
        let entry = inner.entries.get_mut(root)?;
        entry.2 = tick;
        Some(Arc::clone(&entry.0))
    }

    /// Keep `layers` if it fits the byte budget, evicting the least recently
    /// used entries to make room.
    fn insert(&self, root: [u8; 32], layers: V2ProofLayers, bytes: u64) {
        if bytes > V2_PROOF_CACHE_BYTES {
            return;
        }
        let mut inner = self.lock();
        if let Some((_, old_bytes, _)) = inner.entries.remove(&root) {
            inner.bytes = inner.bytes.saturating_sub(old_bytes);
        }
        while inner.bytes.saturating_add(bytes) > V2_PROOF_CACHE_BYTES {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some((_, evicted, _)) = inner.entries.remove(&oldest) {
                inner.bytes = inner.bytes.saturating_sub(evicted);
            }
        }
        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;
        inner.bytes = inner.bytes.saturating_add(bytes);
        inner.entries.insert(root, (layers, bytes, tick));
    }
}

impl V2PeerContext {
    fn file_is_wanted(&self, file_index: u32) -> bool {
        self.file_policy
            .get(&file_index)
            .copied()
            .unwrap_or((true, 1))
            .0
    }

    fn piece_is_wanted(&self, piece: u32) -> bool {
        let Ok(region) = self.piece_map.piece_to_file(piece) else {
            return false;
        };
        !region.pad && self.file_is_wanted(region.file_index)
    }
}

struct V2PieceAssembly {
    region_len: u32,
    received: Vec<bool>,
    received_count: usize,
    pending_blocks: usize,
    discard_when_idle: bool,
    failed: bool,
    data: Option<Vec<u8>>,
    _memory_lease: Option<MemoryLease>,
}

#[derive(Debug)]
struct V2OutstandingRequest {
    length: u32,
    /// Response capacity is reserved before the request reaches the wire and
    /// follows its coordinate until Piece, Reject, Choke, or disconnect.
    _memory_lease: Option<MemoryLease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct V2UploadRequest {
    piece: u32,
    begin: u32,
    length: u32,
}

struct V2LeasedUploadBlock {
    data: Bytes,
    _lease: MemoryLease,
}

enum V2UploadOutcome {
    Ready(V2LeasedUploadBlock),
    RateInterrupted,
    Rejected,
    Failed(anyhow::Error),
}

struct V2UploadCompletion {
    request: V2UploadRequest,
    outcome: V2UploadOutcome,
}

type V2UploadReadTask = BoxFuture<'static, V2UploadCompletion>;

struct V2PieceCompletion {
    piece: u32,
    bytes: u64,
    result: anyhow::Result<()>,
}

struct V2BlockCompletion {
    piece: u32,
    begin: u32,
    result: Result<(), String>,
}

enum V2TransferCompletion {
    BlockStored(V2BlockCompletion),
    BlockAdmissionCancelled { piece: u32, begin: u32 },
    PieceVerified(V2PieceCompletion),
}

type V2TransferTask = BoxFuture<'static, V2TransferCompletion>;

struct V2UploadControlGuard(RateLimitCancellation);

impl Drop for V2UploadControlGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl V2PieceAssembly {
    fn new(
        piece: u32,
        region_len: u32,
        cap_bytes: usize,
        resources: &ResourceGovernor,
    ) -> anyhow::Result<Self> {
        let block_count =
            (region_len as usize).div_ceil(rt_peer_wire::message::MAX_BLOCK_SIZE as usize);
        if block_count == 0 || block_count > V2_MAX_PIECE_BLOCKS {
            anyhow::bail!("v2 piece {piece} has too many blocks");
        }
        let bookkeeping_bytes = u64::try_from(block_count)
            .map_err(|_| anyhow::anyhow!("v2 piece bookkeeping size overflow"))?;
        let mut received = Vec::new();
        received
            .try_reserve_exact(block_count)
            .map_err(|error| anyhow::anyhow!("v2 piece bookkeeping allocation failed: {error}"))?;
        received.resize(block_count, false);
        let (data, memory_lease) = if cap_bytes > 0 && (region_len as usize) <= cap_bytes {
            let bytes = u64::from(region_len).saturating_add(bookkeeping_bytes);
            let lease = resources
                .try_acquire(MemoryClass::PieceAssembly, bytes)
                .ok_or_else(|| {
                    anyhow::anyhow!("v2 piece assembly allocation denied: {bytes} bytes")
                })?;
            let mut data = Vec::new();
            data.try_reserve_exact(region_len as usize)
                .map_err(|error| anyhow::anyhow!("v2 piece assembly allocation failed: {error}"))?;
            data.resize(region_len as usize, 0);
            (Some(data), Some(lease))
        } else {
            let lease = resources
                .try_acquire(MemoryClass::PieceAssembly, bookkeeping_bytes)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "v2 piece bookkeeping allocation denied: {bookkeeping_bytes} bytes"
                    )
                })?;
            (None, Some(lease))
        };
        Ok(Self {
            region_len,
            received,
            received_count: 0,
            pending_blocks: 0,
            discard_when_idle: false,
            failed: false,
            data,
            _memory_lease: memory_lease,
        })
    }

    fn block_index(&self, begin: u32) -> anyhow::Result<usize> {
        let index = begin as usize / rt_peer_wire::message::MAX_BLOCK_SIZE as usize;
        if index >= self.received.len()
            || begin as usize >= self.region_len as usize
            || !(begin as usize).is_multiple_of(rt_peer_wire::message::MAX_BLOCK_SIZE as usize)
        {
            anyhow::bail!("v2 piece block begins outside the piece");
        }
        Ok(index)
    }

    fn accept_block(&mut self, begin: u32, data: &[u8]) -> anyhow::Result<bool> {
        let index = self.block_index(begin)?;
        let expected = (self.region_len as usize)
            .saturating_sub(begin as usize)
            .min(rt_peer_wire::message::MAX_BLOCK_SIZE as usize);
        if data.len() != expected {
            anyhow::bail!(
                "v2 piece block has {} bytes, expected {}",
                data.len(),
                expected
            );
        }
        if self.received[index] {
            if self.data.as_ref().is_some_and(|current| {
                current[begin as usize..begin as usize + data.len()] != *data
            }) {
                anyhow::bail!("conflicting duplicate v2 piece block");
            }
            return Ok(false);
        }
        if let Some(current) = self.data.as_mut() {
            current[begin as usize..begin as usize + data.len()].copy_from_slice(data);
        }
        self.received[index] = true;
        self.received_count += 1;
        Ok(true)
    }

    fn is_complete(&self) -> bool {
        self.received_count == self.received.len()
    }

    fn is_ready(&self) -> bool {
        self.is_complete() && self.pending_blocks == 0 && !self.failed
    }

    /// First block that is neither received nor already requested from this
    /// peer, so a piece can have several blocks on the wire at once.
    fn next_unrequested(
        &self,
        piece: u32,
        outstanding: &HashMap<(u32, u32), V2OutstandingRequest>,
    ) -> Option<u32> {
        self.received
            .iter()
            .enumerate()
            .find_map(|(index, received)| {
                let begin = u32::try_from(index)
                    .ok()?
                    .checked_mul(rt_peer_wire::message::MAX_BLOCK_SIZE)?;
                (!received && !outstanding.contains_key(&(piece, begin))).then_some(begin)
            })
    }
}

/// Running pure-v2 task. It owns the v2 piece map and local have bitmap while
/// the engine remains the owner of registry and durable projection ordering.
pub struct V2TorrentTask {
    info_hash_hex: String,
    meta: Arc<TorrentMetaV2>,
    metainfo_files: Vec<TorrentFileV2>,
    save_root: PathBuf,
    piece_map: Arc<V2PieceMap>,
    storage: MountScheduler,
    registry: Arc<RwLock<SessionRegistry>>,
    db: DbExecutor,
    resources: ResourceGovernor,
    network_budget: GlobalNetworkBudget,
    cmd_rx: mpsc::Receiver<TorrentCmd>,
    events_tx: mpsc::Sender<V2PeerEvent>,
    events_rx: mpsc::Receiver<V2PeerEvent>,
    local_have: Arc<RwLock<Arc<V2Bitmap>>>,
    have_updates: watch::Sender<()>,
    upload_unchoked: watch::Sender<Arc<HashSet<PeerId>>>,
    peers: HashMap<SocketAddr, V2PeerHandle>,
    choker: Choker,
    paused: bool,
    initial_state: TorrentState,
    stopped_announced: bool,
    max_peers: usize,
    torrent_max_peers: Option<usize>,
    file_policy: Arc<HashMap<u32, (bool, i64)>>,
    tracker_tiers: Vec<Vec<TrackerState>>,
    active_tracker_tier: usize,
    private_tracker_key: Option<TrackerKey>,
    tracker_event: TrackerEvent,
    tracker_workers: TrackerWorkers,
    listen_port: u16,
    http_timeout: Duration,
    udp_timeout: Duration,
    min_announce_interval: Option<Duration>,
    known_tracker_peers: HashSet<SocketAddr>,
    tracker_peer_cache_drops: u64,
    assembly_cap_bytes: usize,
    proof_cache: Arc<V2ProofCache>,
    egress_policy: OutboundEgressPolicy,
    _piece_index_memory_lease: Option<MemoryLease>,
    _torrent_metadata_memory_lease: Option<MemoryLease>,
    _file_policy_memory_lease: Option<MemoryLease>,
    _tracker_state_memory_lease: Option<MemoryLease>,
    _tracker_peer_cache_memory_lease: Option<MemoryLease>,
    /// Shared crash-safety settings and counters; inert until attached.
    crash_safety: Arc<CrashSafetyRuntime>,
    /// A finished download is being held until its data is durable. While set
    /// the torrent reports one byte remaining and stays `Downloading`.
    completion_held: std::sync::atomic::AtomicBool,
    /// The save path's filesystem cannot fsync; stop retrying.
    fsync_unsupported: std::sync::atomic::AtomicBool,
}

impl Drop for V2TorrentTask {
    fn drop(&mut self) {
        for peer in self.peers.values() {
            peer.peer_control.cancel();
        }
    }
}

impl V2TorrentTask {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn new(
        meta: TorrentMetaV2,
        save_root: PathBuf,
        paused: bool,
        initial_state: TorrentState,
        registry: Arc<RwLock<SessionRegistry>>,
        db: DbExecutor,
        resources: ResourceGovernor,
        cmd_rx: mpsc::Receiver<TorrentCmd>,
        max_peers: usize,
        piece_assembly_cap_bytes: usize,
        storage_io: StorageIoConfig,
        egress_policy: OutboundEgressPolicy,
        listen_port: u16,
        http_timeout_secs: u64,
        udp_timeout_secs: u64,
        min_interval_secs: u64,
        network_budget: GlobalNetworkBudget,
        piece_index_memory_lease: Option<MemoryLease>,
        torrent_metadata_memory_lease: Option<MemoryLease>,
    ) -> Self {
        let info_hash_hex = hex::encode(meta.info_hash_v2);
        let metainfo_files = meta.files.clone();
        let metainfo_trackers = meta.all_trackers();
        let mut tracker_tiers = tracker_tiers_from_meta_v2(&meta);
        let tracker_restore = db
            .run("restore_v2_tracker_state", {
                let info_hash = info_hash_hex.clone();
                move |db| {
                    let (count, bytes) = rt_db::torrent_tracker_snapshot_size(db, &info_hash)
                        .map_err(|error| error.to_string())?;
                    crate::engine::validate_tracker_snapshot_size(count, bytes)?;
                    let rows = rt_db::list_torrent_trackers(db, &info_hash)
                        .map_err(|error| error.to_string())?;
                    let persisted_trackers = rt_db::torrent_tracker_override(db, &info_hash)
                        .map_err(|error| error.to_string())?;
                    Ok((rows, persisted_trackers))
                }
            })
            .await;
        match tracker_restore {
            Ok((rows, persisted_trackers)) => {
                if persisted_trackers
                    .as_ref()
                    .is_some_and(|trackers| trackers.as_slice() != metainfo_trackers.as_slice())
                {
                    tracker_tiers =
                        tracker_tiers_from_urls(persisted_trackers.as_deref().unwrap_or_default());
                }
                restore_tracker_state_from_rows(&mut tracker_tiers, &rows);
            }
            Err(error) => {
                warn!(
                    component = "db",
                    operation = "restore_v2_tracker_state",
                    torrent = %info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to restore pure-v2 tracker state; using fresh tracker session"
                );
            }
        }
        let tracker_state_memory_lease =
            reserve_v2_tracker_state_memory(&resources, &tracker_tiers);
        if tracker_state_memory_lease.is_none() && !tracker_tiers.is_empty() {
            warn!(
                component = "memory",
                operation = "reserve_v2_tracker_state",
                torrent = %info_hash_hex,
                result = "disabled",
                "pure-v2 tracker state disabled because its memory budget was unavailable"
            );
            tracker_tiers.clear();
        }
        let private_tracker_key = if meta.private {
            first_tracker_key(&tracker_tiers)
        } else {
            None
        };
        let meta = Arc::new(meta);
        let spans = meta
            .files
            .iter()
            .map(|file| V2FileSpan {
                file_index: file.index,
                path: file.path.clone(),
                piece_offset: file.piece_offset,
                length: file.length,
                pad: file.pad,
            })
            .collect();
        let piece_map = Arc::new(
            V2PieceMap::new(meta.piece_length, spans)
                .expect("metainfo parser rejects invalid v2 piece maps"),
        );
        let peer_event_capacity = max_peers.clamp(64, 512);
        let (events_tx, events_rx) = mpsc::channel(peer_event_capacity);
        let local_have = Arc::new(RwLock::new(Arc::new(V2Bitmap::new(
            piece_map.piece_count as usize,
        ))));
        let (have_updates, _) = watch::channel(());
        let (upload_unchoked, _) = watch::channel(Arc::new(HashSet::<PeerId>::new()));
        let storage = MountScheduler::new_for_path(
            StorageRootId::new(),
            &save_root,
            &SchedulerConfig {
                profile: StorageProfile::Unknown,
                resources: Some(resources.clone()),
                storage_io,
                ..Default::default()
            },
        );
        let mut task = Self {
            info_hash_hex,
            meta,
            metainfo_files,
            save_root,
            piece_map,
            storage,
            registry,
            db,
            resources,
            network_budget,
            cmd_rx,
            events_tx,
            events_rx,
            local_have,
            have_updates,
            upload_unchoked,
            peers: HashMap::new(),
            choker: Choker::new(DEFAULT_MAX_UNCHOKED),
            paused,
            initial_state,
            stopped_announced: paused,
            max_peers,
            torrent_max_peers: None,
            file_policy: Arc::new(HashMap::new()),
            tracker_tiers,
            active_tracker_tier: 0,
            private_tracker_key,
            tracker_event: TrackerEvent::Started,
            tracker_workers: TrackerWorkers::announce_only(),
            listen_port,
            http_timeout: Duration::from_secs(http_timeout_secs.max(1)),
            udp_timeout: Duration::from_secs(udp_timeout_secs.max(1)),
            min_announce_interval: (min_interval_secs > 0)
                .then(|| Duration::from_secs(min_interval_secs)),
            known_tracker_peers: HashSet::new(),
            tracker_peer_cache_drops: 0,
            assembly_cap_bytes: piece_assembly_cap_bytes,
            proof_cache: Arc::new(V2ProofCache::default()),
            egress_policy,
            _piece_index_memory_lease: piece_index_memory_lease,
            _torrent_metadata_memory_lease: torrent_metadata_memory_lease,
            _file_policy_memory_lease: None,
            _tracker_state_memory_lease: tracker_state_memory_lease,
            _tracker_peer_cache_memory_lease: None,
            crash_safety: CrashSafetyRuntime::inert(),
            completion_held: std::sync::atomic::AtomicBool::new(false),
            fsync_unsupported: std::sync::atomic::AtomicBool::new(false),
        };
        let (known_tracker_peers, tracker_peer_cache_memory_lease) =
            prepare_tracker_peer_cache(&task.resources, task.max_peers);
        task.known_tracker_peers = known_tracker_peers;
        task._tracker_peer_cache_memory_lease = tracker_peer_cache_memory_lease;
        task.reset_default_file_policy();
        if let Err(error) = task.apply_file_policy_from_db().await {
            warn!(
                component = "torrent_v2",
                operation = "restore_file_policy",
                torrent = %task.info_hash_hex,
                result = "error",
                error = %error,
                "failed to restore pure-v2 file policy; using metainfo paths and default wanted state"
            );
            task.reset_default_file_policy();
        }
        task
    }

    /// Share the engine's crash-safety runtime with this task.
    pub(crate) fn attach_crash_safety(&mut self, runtime: Arc<CrashSafetyRuntime>) {
        self.crash_safety = runtime;
    }

    /// Whether a finished download may be reported complete now.
    ///
    /// Pure-v2 torrents re-hash every file at startup, so they never trust a
    /// stale resume record; this guards the *hand-off*: nothing may see the
    /// torrent as complete before its data has been fsynced and, when
    /// configured (or forced by a weak mount), re-read from disk and verified.
    /// Returns `true` when the completion may be released.
    async fn completion_may_release(&self) -> bool {
        use std::sync::atomic::Ordering;
        let location = self.crash_safety.for_location(&self.save_root);
        let verify = location.completion_verify;
        // Read-back verification implies the barrier, exactly as in the v1 task.
        if !location.resolved.completion_gate && verify == rt_config::CompletionVerifyMode::Off {
            self.completion_held.store(false, Ordering::Relaxed);
            return true;
        }
        let barrier_ok = if self.fsync_unsupported.load(Ordering::Relaxed) {
            if self.completion_held.swap(false, Ordering::Relaxed) {
                self.crash_safety.record_gate_released();
            }
            true
        } else {
            self.apply_barrier_result(self.storage.sync_all_open_files().await)
        };
        if !barrier_ok {
            return false;
        }
        match verify {
            rt_config::CompletionVerifyMode::Off => true,
            mode => self.verify_completed_download(mode).await,
        }
    }

    /// Re-read the finished download from disk and check it against the
    /// metainfo. `full` checks every file; `sample` checks a deterministic
    /// subset of files covering at least the configured share of the bytes
    /// (a v2 file is verified as a whole against its Merkle root, so the unit
    /// of sampling is the file). Returns whether the torrent may be released.
    ///
    /// A failure clears the affected pieces so they are downloaded again; a
    /// failed sample escalates to a full verification, since it proves storage
    /// returned data other than what was written.
    async fn verify_completed_download(&self, mode: rt_config::CompletionVerifyMode) -> bool {
        use std::sync::atomic::Ordering;
        let sampled = mode == rt_config::CompletionVerifyMode::Sample;
        // Show the hold while the (possibly long) read-back runs.
        if !self.completion_held.swap(true, Ordering::Relaxed) {
            self.crash_safety.record_gate_started();
        }
        let _ = self.persist_runtime(None, None).await;

        let mut dropped = 0usize;
        for file in self.meta.files.iter().filter(|file| !file.pad) {
            if self
                .storage
                .drop_page_cache(&file.path.resolve(&self.save_root))
                .await
            {
                dropped += 1;
            }
        }
        debug!(
            component = "torrent_v2",
            operation = "completion_verify",
            torrent = %self.info_hash_hex,
            sampled,
            page_cache_dropped_files = dropped,
            "starting read-back verification of the completed download"
        );

        let sample_percent = self
            .crash_safety
            .for_location(&self.save_root)
            .resolved
            .completion_verify_sample_percent;
        let selection = if sampled {
            let sizes: Vec<(u32, u64)> = self
                .meta
                .files
                .iter()
                .filter(|file| !file.pad && file.length > 0)
                .map(|file| (file.index, file.length))
                .collect();
            let mut seed_bytes = [0u8; 8];
            for (slot, byte) in seed_bytes.iter_mut().zip(self.meta.info_hash_v2.iter()) {
                *slot = *byte;
            }
            select_v2_sample_files(&sizes, sample_percent, u64::from_le_bytes(seed_bytes))
        } else {
            None
        };
        let (mut failed, mut checked) = self.verify_v2_files(selection.as_deref()).await;
        // A failed sample: check everything, so every bad file is found now.
        if sampled && !failed.is_empty() && selection.is_some() {
            let (all_failed, all_checked) = self.verify_v2_files(None).await;
            failed = all_failed;
            checked = all_checked;
        }
        let failed_pieces = self.clear_v2_files_from_have(&failed).await;
        self.crash_safety
            .record_completion_verify(checked as u64, failed_pieces as u64);

        let mode_name = if sampled && selection.is_some() {
            "sample"
        } else {
            "full"
        };
        if failed.is_empty() {
            self.completion_held.store(false, Ordering::Relaxed);
            self.crash_safety.record_gate_released();
            self.append_v2_event(
                "completion_verified",
                "completed download was read back from disk and verified",
                serde_json::json!({ "mode": mode_name, "files_checked": checked }),
            )
            .await;
            true
        } else {
            self.completion_held.store(false, Ordering::Relaxed);
            self.crash_safety.record_gate_dropped();
            self.notify_local_have_changed();
            self.append_v2_event(
                "completion_verify_failed",
                "read-back verification found data that does not match; the bad pieces will be downloaded again",
                serde_json::json!({
                    "level": "error",
                    "mode": mode_name,
                    "files_checked": checked,
                    "files_failed": failed.len(),
                    "pieces_invalidated": failed_pieces,
                }),
            )
            .await;
            false
        }
    }

    /// Verify the named files (all non-pad files when `only` is `None`).
    /// Returns the indexes of files that did not verify and how many were
    /// checked.
    async fn verify_v2_files(&self, only: Option<&[u32]>) -> (Vec<u32>, usize) {
        let files: Vec<V2FileHash> = self
            .meta
            .files
            .iter()
            .filter(|file| !file.pad)
            .filter(|file| only.is_none_or(|only| only.contains(&file.index)))
            .map(|file| V2FileHash {
                file_index: file.index,
                path: file.path.clone(),
                length: file.length,
                pieces_root: file.pieces_root,
            })
            .collect();
        let verifier = V2FileVerifier::new(&self.save_root, &self.storage, &files);
        let results = verifier.verify_all().await;
        let checked = results.len();
        let failed = results
            .into_iter()
            .filter(|(_, result)| !matches!(result, VerifyResult::Valid))
            .map(|(index, _)| index)
            .collect();
        (failed, checked)
    }

    /// Mark every piece of the given files as not held. Returns how many
    /// pieces were cleared.
    async fn clear_v2_files_from_have(&self, files: &[u32]) -> usize {
        if files.is_empty() {
            return 0;
        }
        let mut cleared = 0usize;
        let mut have = self.local_have.write().await;
        let have = Arc::make_mut(&mut have);
        if let Ok(ranges) = self.piece_map.piece_ranges_for_file_indices(&{
            let mut sorted = files.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            sorted
        }) {
            for (first, last) in ranges {
                for piece in first..last {
                    if have.get(piece as usize) {
                        have.set(piece as usize, false);
                        cleared += 1;
                    }
                }
            }
        }
        cleared
    }

    /// Append a per-torrent event to the session event log. Best effort.
    async fn append_v2_event(&self, kind: &'static str, message: &str, payload: serde_json::Value) {
        let event = rt_db::SessionEventRow {
            event_id: None,
            occurred_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .min(i64::MAX as u64) as i64,
            info_hash: Some(self.info_hash_hex.clone()),
            kind: kind.to_owned(),
            message: Some(message.to_owned()),
            payload: payload.to_string(),
        };
        let result = self
            .db
            .run("append_v2_torrent_event", move |db| {
                let tx = db.transaction().map_err(|error| error.to_string())?;
                rt_db::append_session_event_in_tx(&tx, &event)
                    .map_err(|error| error.to_string())?;
                tx.commit().map_err(|error| error.to_string())
            })
            .await;
        if let Err(error) = result {
            debug!(
                component = "db",
                operation = "append_v2_torrent_event",
                torrent = %self.info_hash_hex,
                kind,
                error = %error,
                "could not record torrent event"
            );
        }
    }

    /// After a recheck: a torrent already reported complete that now lacks
    /// wanted pieces is an integrity regression. Records the event with the
    /// affected files; repair downloads the missing pieces in place.
    async fn report_v2_recheck(&self, have: &V2Bitmap, trigger: &'static str) {
        let completed = self
            .registry
            .read()
            .await
            .get(&self.info_hash_hex)
            .is_some_and(|entry| entry.completed_at.is_some());
        if !completed {
            return;
        }
        let missing: Vec<u32> = (0..self.piece_map.piece_count)
            .filter(|piece| self.piece_is_wanted(*piece) && !have.get(*piece as usize))
            .collect();
        if missing.is_empty() {
            return;
        }
        let mut files: Vec<String> = Vec::new();
        for piece in missing.iter().take(MAX_V2_REGRESSION_PIECES) {
            if let Ok(region) = self.piece_map.piece_to_file(*piece) {
                let path = region.path.resolve(&self.save_root).display().to_string();
                if files.len() < MAX_V2_REGRESSION_FILES && !files.contains(&path) {
                    files.push(path);
                }
            }
        }
        self.crash_safety.record_integrity_regression();
        warn!(
            component = "torrent_v2",
            operation = "integrity_regression",
            torrent = %self.info_hash_hex,
            trigger,
            missing_pieces = missing.len(),
            "a torrent already reported complete has pieces that no longer verify; repairing in place"
        );
        self.append_v2_event(
            "integrity_regression",
            "a completed torrent has pieces that no longer verify; missing data will be downloaded again in place",
            serde_json::json!({
                "level": "error",
                "trigger": trigger,
                "invalid_pieces": missing.len(),
                "sample_pieces": missing.iter().take(MAX_V2_REGRESSION_PIECES).collect::<Vec<_>>(),
                "affected_files": files,
                "note": "repair rewrites the existing files in place, so hardlinks to them are healed; copies made earlier are not",
            }),
        )
        .await;
    }

    /// Decide from the outcome of a payload barrier. Split from the sync itself
    /// so every branch is testable without a failing disk.
    fn apply_barrier_result(&self, result: Result<(), rt_storage::StorageError>) -> bool {
        use std::sync::atomic::Ordering;
        match result {
            Ok(()) => {
                if self.completion_held.swap(false, Ordering::Relaxed) {
                    self.crash_safety.record_gate_released();
                }
                true
            }
            Err(error) if error.is_operation_unsupported() => {
                self.fsync_unsupported.store(true, Ordering::Relaxed);
                self.crash_safety.note_fsync_unsupported(&self.save_root);
                warn!(
                    component = "torrent_v2",
                    operation = "completion_gate",
                    torrent = %self.info_hash_hex,
                    error = %error,
                    "this filesystem cannot fsync; completion is released without a data barrier"
                );
                if self.completion_held.swap(false, Ordering::Relaxed) {
                    self.crash_safety.record_gate_released();
                }
                true
            }
            Err(error) => {
                if !self.completion_held.swap(true, Ordering::Relaxed) {
                    self.crash_safety.record_gate_started();
                }
                warn!(
                    component = "torrent_v2",
                    operation = "completion_gate",
                    torrent = %self.info_hash_hex,
                    error = %error,
                    "download complete but its data is not yet durable; holding completion and retrying"
                );
                false
            }
        }
    }

    pub async fn run(mut self) {
        let new_have = self.recheck_files().await;
        self.report_v2_recheck(&new_have, "startup").await;
        *self.local_have.write().await = Arc::new(new_have);
        self.notify_local_have_changed();

        let startup_state = if self.paused {
            match self.initial_state {
                TorrentState::Error => TorrentState::Error,
                TorrentState::Queued => TorrentState::Queued,
                TorrentState::Stopped => TorrentState::Stopped,
                _ => TorrentState::Paused,
            }
        } else if self.is_complete().await {
            TorrentState::Seeding
        } else {
            TorrentState::Downloading
        };
        if let Err(error) = self.persist_runtime(Some(startup_state), None).await {
            warn!(
                component = "torrent_v2",
                operation = "startup_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist pure-v2 startup state"
            );
            return;
        }

        let mut flush_tick = interval(Duration::from_secs(5));
        let mut tracker_tick = interval(Duration::from_secs(5));
        let mut choke_tick = interval(V2_CHOKE_INTERVAL);
        loop {
            tokio::select! {
                command = self.cmd_rx.recv() => {
                    let Some(command) = command else {
                        self.tracker_workers.cancel();
                        self.announce_stopped_with_control_deadline().await;
                        self.shutdown_peers().await;
                        let _ = self.persist_runtime(None, None).await;
                        break;
                    };
                    if self.handle_command(command).await {
                        break;
                    }
                }
                Some(event) = self.events_rx.recv() => {
                    self.handle_peer_event(event).await;
                }
                _ = choke_tick.tick() => {
                    if !self.paused {
                        self.run_choker().await;
                    }
                }
                Some(result) = self.tracker_workers.recv() => {
                    self.handle_tracker_result(result).await;
                }
                _ = flush_tick.tick() => {
                    if !self.paused {
                        let held = self.completion_held.load(std::sync::atomic::Ordering::Relaxed);
                        let target = if held && self.is_complete().await {
                            self.completion_may_release()
                                .await
                                .then_some(TorrentState::Seeding)
                        } else {
                            None
                        };
                        let _ = self.persist_runtime(target, None).await;
                    }
                }
                _ = tracker_tick.tick() => {
                    if !self.paused {
                        self.start_due_tracker_announces().await;
                    }
                }
            }
        }
        self.reject_pending_commands();
    }

    async fn handle_command(&mut self, command: TorrentCmd) -> bool {
        match command {
            TorrentCmd::Shutdown => {
                self.tracker_workers.cancel();
                self.announce_stopped_with_control_deadline().await;
                self.shutdown_peers().await;
                let _ = self.persist_runtime(None, None).await;
                true
            }
            TorrentCmd::Pause { reply } => {
                let was_paused = self.paused;
                self.paused = true;
                self.tracker_workers.cancel();
                self.shutdown_peers().await;
                let result = self.persist_runtime(Some(TorrentState::Paused), None).await;
                if result.is_err() {
                    self.paused = was_paused;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                if self.paused {
                    self.announce_stopped_with_control_deadline().await;
                }
                false
            }
            TorrentCmd::Resume { reply } => {
                let result = self.resume_runtime(reply.is_some()).await;
                if let Some(reply) = reply {
                    let _ = reply.send(result.clone());
                }
                if result.is_ok() {
                    self.recheck_and_set_state().await;
                }
                false
            }
            TorrentCmd::QuiesceForStorageMove { reply } => {
                let was_paused = self.paused;
                self.paused = true;
                self.tracker_workers.cancel();
                self.shutdown_peers().await;
                let result = self
                    .persist_runtime(Some(TorrentState::Paused), None)
                    .await
                    .map(|()| was_paused);
                if result.is_err() {
                    self.paused = was_paused;
                }
                let should_announce_stopped = result.as_ref().is_ok_and(|was_paused| !*was_paused);
                let _ = reply.send(result);
                if should_announce_stopped {
                    self.announce_stopped_with_control_deadline().await;
                }
                false
            }
            TorrentCmd::ResumeAfterStorageMove {
                new_save_root,
                resume_paused,
                reply,
            } => {
                if let Some(new_root) = new_save_root {
                    self.save_root = new_root.clone();
                    self.storage = MountScheduler::new_for_path(
                        StorageRootId::new(),
                        &new_root,
                        &SchedulerConfig {
                            profile: StorageProfile::Unknown,
                            resources: Some(self.resources.clone()),
                            storage_io: self.storage.io_config().clone(),
                            ..Default::default()
                        },
                    );
                }
                if resume_paused {
                    let _ = reply.send(Ok(()));
                } else {
                    self.paused = false;
                    let result = self.resume_runtime(true).await;
                    let _ = reply.send(result.clone());
                    if result.is_ok() {
                        self.recheck_and_set_state().await;
                    }
                }
                false
            }
            TorrentCmd::Recheck { .. } => {
                self.recheck_and_set_state().await;
                false
            }
            TorrentCmd::NewPeers(peers) => {
                if !self.paused {
                    self.connect_peers(peers, PeerSource::Dht).await;
                }
                false
            }
            TorrentCmd::PriorityPeers(peers) => {
                if !self.paused {
                    // Private torrents have no tracker/DHT/PEX peer source.
                    // An explicit peer supplied by a client is therefore the
                    // authority that admits this address for the connection
                    // and for later reconnects.
                    if self.meta.private {
                        self.remember_tracker_peers(&peers);
                    }
                    self.connect_priority_peers(peers).await;
                }
                false
            }
            TorrentCmd::AcceptPeer {
                stream,
                peer_addr,
                handshake,
                peer_permit,
            } => {
                if !self.paused {
                    self.accept_peer(stream, peer_addr, handshake, peer_permit)
                        .await;
                }
                false
            }
            TorrentCmd::AcceptUtpPeer {
                stream,
                peer_addr,
                handshake,
                peer_permit,
            } => {
                if !self.paused {
                    self.accept_utp_peer(*stream, peer_addr, handshake, peer_permit)
                        .await;
                }
                false
            }
            TorrentCmd::BanPeer(peer) => {
                self.remove_peer(peer).await;
                false
            }
            TorrentCmd::EvictBannedPeers => {
                let banned = {
                    let registry = self.registry.read().await;
                    self.peers
                        .keys()
                        .copied()
                        .filter(|peer| registry.is_peer_banned(*peer))
                        .collect::<Vec<_>>()
                };
                for peer in banned {
                    self.remove_peer(peer).await;
                }
                false
            }
            TorrentCmd::GetPeers { reply } => {
                let _ = reply.send(self.peer_snapshots_unbounded());
                false
            }
            TorrentCmd::GetPeerSnapshotCount { reply } => {
                let _ = reply.send(self.peers.len());
                false
            }
            TorrentCmd::GetPeerSnapshots { max_entries, reply } => {
                let result = if max_entries > crate::torrent_task::MAX_PEER_SNAPSHOT_ITEMS {
                    Err(format!(
                        "requested peer snapshot limit {max_entries} exceeds {}",
                        crate::torrent_task::MAX_PEER_SNAPSHOT_ITEMS
                    ))
                } else if self.peers.len() > max_entries {
                    Err(format!(
                        "torrent peer snapshot contains {} peers; maximum is {max_entries}",
                        self.peers.len()
                    ))
                } else {
                    Ok(self.peer_snapshots_unbounded())
                };
                let _ = reply.send(result);
                false
            }
            TorrentCmd::GetWebseeds { reply } => {
                let _ = reply.send(Vec::<EngineWebseedSnapshot>::new());
                false
            }
            TorrentCmd::GetRuntimeStats { reply } => {
                let _ = reply.send(self.runtime_stats());
                false
            }
            TorrentCmd::ReloadFilePolicy { reply } => {
                let result = self.apply_file_policy_from_db().await;
                if result.is_ok() {
                    self.recheck_and_set_state().await;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                false
            }
            TorrentCmd::UpdateLimits { limits, reply } => {
                self.apply_limits(&limits);
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
                false
            }
            TorrentCmd::UpdateTrackers { trackers, reply } => {
                let result = self.apply_tracker_urls(trackers).await;
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                false
            }
            TorrentCmd::Reannounce => {
                self.tracker_event = TrackerEvent::Empty;
                self.schedule_active_tracker_tier_now();
                false
            }
            TorrentCmd::CancelJob { .. } | TorrentCmd::UpdatePeerExchange(_) => false,
        }
    }

    fn apply_limits(&mut self, limits: &EngineTorrentLimits) {
        self.torrent_max_peers = limits.max_connections.and_then(|value| {
            usize::try_from(value)
                .ok()
                .filter(|connections| *connections > 0)
        });
    }

    fn reset_default_file_policy(&mut self) {
        self.file_policy = Arc::new(
            self.meta
                .files
                .iter()
                .map(|file| (file.index, (!file.pad, if file.pad { 0 } else { 1 })))
                .collect(),
        );
    }

    fn reserve_v2_file_policy_memory(
        &self,
        files: &[TorrentFileV2],
    ) -> Result<Option<MemoryLease>, String> {
        let mut bytes = files
            .len()
            .saturating_mul(std::mem::size_of::<TorrentFileV2>());
        for file in files {
            bytes = bytes.saturating_add(
                file.path
                    .components()
                    .len()
                    .saturating_mul(std::mem::size_of::<String>()),
            );
            for component in file.path.components() {
                bytes = bytes.saturating_add(component.capacity());
            }
        }
        if bytes == 0 {
            return Ok(None);
        }
        let bytes = u64::try_from(bytes)
            .map_err(|_| "pure-v2 file policy memory estimate overflows u64".to_owned())?;
        self.resources
            .try_acquire(MemoryClass::Metadata, bytes)
            .map(Some)
            .ok_or_else(|| format!("pure-v2 file policy allocation of {bytes} bytes denied"))
    }

    fn reserve_v2_file_policy_workspace_memory(&self) -> Result<Option<MemoryLease>, String> {
        let bytes = v2_file_graph_memory_bytes(&self.metainfo_files, self.metainfo_files.len());
        if bytes == 0 {
            return Ok(None);
        }
        let bytes = u64::try_from(bytes).map_err(|_| {
            "pure-v2 file-policy workspace memory estimate overflows u64".to_owned()
        })?;
        self.resources
            .try_acquire(MemoryClass::Metadata, bytes)
            .map(Some)
            .ok_or_else(|| {
                format!("pure-v2 file-policy workspace allocation of {bytes} bytes denied")
            })
    }

    async fn apply_file_policy_from_db(&mut self) -> Result<(), String> {
        let info_hash = self.info_hash_hex.clone();
        let rows = self
            .db
            .run("load_v2_file_policy", move |db| {
                rt_db::list_torrent_files(db, &info_hash)
                    .map_err(|error| format!("loading pure-v2 file policy: {error}"))
            })
            .await?;
        // Admit the temporary graph before cloning it and constructing the
        // replacement V2PieceMap. The retained-policy lease below covers the
        // post-swap lifetime; this lease covers the reload workspace peak.
        let _file_policy_workspace_memory_lease = self.reserve_v2_file_policy_workspace_memory()?;
        let mut effective_files = self.metainfo_files.clone();
        let mut policy = HashMap::with_capacity(rows.len());
        for row in rows {
            let file_index = u32::try_from(row.file_index).map_err(|_| {
                format!(
                    "persisted pure-v2 file policy has invalid file index {}",
                    row.file_index
                )
            })?;
            let Some(file) = effective_files
                .iter_mut()
                .find(|file| file.index == file_index)
            else {
                return Err(format!(
                    "persisted pure-v2 file policy references unknown file index {file_index}"
                ));
            };
            file.path = rt_path::SafeRelPath::from_slash_separated(&row.path, cfg!(windows))
                .map_err(|error| {
                    format!(
                        "invalid persisted pure-v2 file path {:?}: {error}",
                        row.path
                    )
                })?;
            if !(0..=2).contains(&row.priority) {
                return Err(format!(
                    "persisted pure-v2 file policy has invalid priority {} for file {file_index}",
                    row.priority
                ));
            }
            if policy
                .insert(file_index, (row.wanted && !file.pad, row.priority))
                .is_some()
            {
                return Err(format!(
                    "persisted pure-v2 file policy contains duplicate file index {file_index}"
                ));
            }
        }

        crate::engine::validate_file_path_projection(
            effective_files.iter().map(|file| file.path.as_display()),
        )?;
        for file in &effective_files {
            policy
                .entry(file.index)
                .or_insert((!file.pad, if file.pad { 0 } else { 1 }));
        }

        let policy_changed = self.file_policy.as_ref() != &policy;
        let current_files = self.meta.files.clone();
        let paths_changed = current_files.len() != effective_files.len()
            || current_files
                .iter()
                .zip(&effective_files)
                .any(|(current, effective)| {
                    current.index != effective.index || current.path != effective.path
                });
        if paths_changed {
            let mut changed_file_indices = current_files
                .iter()
                .filter_map(|current| {
                    effective_files
                        .iter()
                        .find(|effective| effective.index == current.index)
                        .filter(|effective| effective.path != current.path)
                        .map(|_| current.index)
                })
                .collect::<Vec<_>>();
            changed_file_indices.sort_unstable();
            changed_file_indices.dedup();
            let changed_piece_ranges = self
                .piece_map
                .piece_ranges_for_file_indices(&changed_file_indices)
                .map_err(|error| format!("invalidating renamed pure-v2 file pieces: {error}"))?;
            let piece_map = Arc::new(
                V2PieceMap::new(
                    self.meta.piece_length,
                    effective_files
                        .iter()
                        .map(|file| V2FileSpan {
                            file_index: file.index,
                            path: file.path.clone(),
                            piece_offset: file.piece_offset,
                            length: file.length,
                            pad: file.pad,
                        })
                        .collect(),
                )
                .map_err(|error| format!("rebuilding pure-v2 file map: {error}"))?,
            );
            let file_policy_memory_lease = self.reserve_v2_file_policy_memory(&effective_files)?;
            self.shutdown_peers().await;
            {
                let mut have = self.local_have.write().await;
                let have = Arc::make_mut(&mut *have);
                for (first, last) in changed_piece_ranges {
                    for piece in first..last {
                        have.set(piece as usize, false);
                    }
                }
            }
            Arc::make_mut(&mut self.meta).files = effective_files;
            self.piece_map = piece_map;
            self._file_policy_memory_lease = file_policy_memory_lease;
        } else if policy_changed {
            // Peer contexts hold an immutable snapshot of this policy.  Tear
            // them down so reconnects cannot continue requesting files whose
            // wanted/priority state has just changed.
            self.shutdown_peers().await;
        }
        self.file_policy = Arc::new(policy);
        Ok(())
    }

    fn piece_is_wanted(&self, piece: u32) -> bool {
        let Ok(region) = self.piece_map.piece_to_file(piece) else {
            return false;
        };
        !region.pad
            && self
                .file_policy
                .get(&region.file_index)
                .copied()
                .unwrap_or((true, 1))
                .0
    }

    fn notify_local_have_changed(&self) {
        let _ = self.have_updates.send(());
    }

    fn schedule_trackers_now(&mut self) {
        if self.meta.private {
            if let Some(key) = self
                .private_tracker_key
                .or_else(|| first_tracker_key(&self.tracker_tiers))
            {
                if let Some(tracker) = self
                    .tracker_tiers
                    .get_mut(key.0)
                    .and_then(|tier| tier.get_mut(key.1))
                {
                    tracker.schedule_immediate();
                }
            }
        } else {
            for tier in &mut self.tracker_tiers {
                for tracker in tier {
                    tracker.schedule_immediate();
                }
            }
        }
    }

    fn schedule_active_tracker_tier_now(&mut self) {
        if self.tracker_tiers.is_empty() {
            return;
        }
        let tier_index = self
            .active_tracker_tier
            .min(self.tracker_tiers.len().saturating_sub(1));
        for (selected_tier, tracker_index) in tracker_keys_for_announce(
            &self.tracker_tiers,
            tier_index,
            self.meta.private,
            self.private_tracker_key,
        ) {
            if let Some(tracker) = self
                .tracker_tiers
                .get_mut(selected_tier)
                .and_then(|tier| tier.get_mut(tracker_index))
            {
                tracker.schedule_immediate();
            }
        }
    }

    async fn apply_tracker_urls(&mut self, trackers: Vec<String>) -> Result<(), String> {
        let tracker_tiers = tracker_tiers_from_urls(&trackers);
        let tracker_state_memory_lease =
            reserve_v2_tracker_state_memory(&self.resources, &tracker_tiers);
        if !tracker_tiers.is_empty() && tracker_state_memory_lease.is_none() {
            return Err("pure-v2 tracker state allocation denied".to_owned());
        }
        self.tracker_workers.cancel();
        if self.meta.private {
            self.disconnect_private_tracker_peers().await;
        }
        self.known_tracker_peers.clear();
        self.tracker_tiers = tracker_tiers;
        self.private_tracker_key = if self.meta.private {
            first_tracker_key(&self.tracker_tiers)
        } else {
            None
        };
        self.active_tracker_tier = self.private_tracker_key.map_or(0, |(tier, _)| tier);
        self.tracker_event = TrackerEvent::Empty;
        self._tracker_state_memory_lease = tracker_state_memory_lease;
        self.schedule_trackers_now();
        {
            let mut registry = self.registry.write().await;
            if let Some(mut entry) = registry.get_mut(&self.info_hash_hex) {
                entry.tracker_message = None;
            };
        }
        self.persist_tracker_state().await
    }

    async fn start_due_tracker_announces(&mut self) {
        if self.tracker_tiers.is_empty() {
            return;
        }
        let tier_idx = self
            .active_tracker_tier
            .min(self.tracker_tiers.len().saturating_sub(1));
        let available = self.tracker_workers.available();
        if available == 0 {
            return;
        }
        let specs = tracker_keys_for_announce(
            &self.tracker_tiers,
            tier_idx,
            self.meta.private,
            self.private_tracker_key,
        )
        .into_iter()
        .filter_map(|key @ (selected_tier, tracker_index)| {
            let tracker = self.tracker_tiers.get(selected_tier)?.get(tracker_index)?;
            (tracker.is_due() && !self.tracker_workers.contains(key)).then(|| TrackerAnnounceSpec {
                key,
                url: tracker.url.clone(),
                tracker_id: tracker.tracker_id.clone(),
                event: self.tracker_event,
            })
        })
        .take(available)
        .collect::<Vec<_>>();
        if specs.is_empty() {
            return;
        }
        let context = self.tracker_announce_context().await;
        self.tracker_workers.start(specs, context);
    }

    async fn tracker_announce_context(&self) -> TrackerAnnounceContext {
        let (uploaded, downloaded) = self
            .registry
            .read()
            .await
            .get(&self.info_hash_hex)
            .map(|entry| (entry.stats.uploaded, entry.stats.downloaded))
            .unwrap_or((0, 0));
        let have = self.local_have.read().await.clone();
        TrackerAnnounceContext {
            info_hash: self.meta.info_hash_v2[..20]
                .try_into()
                .expect("v2 info hash truncation is 20 bytes"),
            uploaded,
            downloaded,
            left: self.bytes_left(have.as_ref()),
            listen_port: self.listen_port,
            http_timeout: self.http_timeout,
            udp_timeout: self.udp_timeout,
            numwant: protocol_numwant(self.peer_capacity()),
            egress_policy: self.egress_policy,
            resources: self.resources.clone(),
        }
    }

    async fn handle_tracker_result(&mut self, result: TrackerAnnounceResult) {
        self.tracker_workers.complete(result.key, result.generation);
        if !self.tracker_workers.is_current(result.generation) {
            return;
        }
        let (tier_idx, tracker_idx) = result.key;
        let Some(tracker) = self
            .tracker_tiers
            .get_mut(tier_idx)
            .and_then(|tier| tier.get_mut(tracker_idx))
        else {
            return;
        };
        let failed = result.response.is_err();
        match result.response {
            Ok(response) => {
                let peers = response
                    .peers
                    .iter()
                    .map(|peer| peer.addr)
                    .collect::<Vec<_>>();
                tracker.on_success_with_min_interval(&response, self.min_announce_interval);
                self.tracker_event =
                    v2_tracker_event_after_success(self.tracker_event, result.event);
                let _ = self.persist_tracker_state().await;
                if !peers.is_empty() && !self.paused {
                    self.remember_tracker_peers(&peers);
                    self.connect_peers(peers, PeerSource::Tracker).await;
                }
            }
            Err(error) => {
                tracker.on_failure(error);
                let _ = self.persist_tracker_state().await;
            }
        }
        if self.meta.private {
            if failed {
                self.advance_private_tracker_after_failure((tier_idx, tracker_idx))
                    .await;
            }
        } else if failed
            && self.tracker_tiers.get(tier_idx).is_some_and(|tier| {
                !tier.is_empty()
                    && tier
                        .iter()
                        .all(|tracker| matches!(tracker.status, TrackerStatus::Error(_)))
            })
        {
            self.active_tracker_tier = self
                .active_tracker_tier
                .saturating_add(1)
                .min(self.tracker_tiers.len().saturating_sub(1));
        }
    }

    async fn advance_private_tracker_after_failure(&mut self, failed: TrackerKey) {
        if self.private_tracker_key != Some(failed) {
            return;
        }
        let Some(next) = next_tracker_key(&self.tracker_tiers, failed) else {
            return;
        };
        if next == failed {
            return;
        }

        self.tracker_workers.cancel();
        self.disconnect_private_tracker_peers().await;
        self.private_tracker_key = Some(next);
        self.active_tracker_tier = next.0;
        if let Some(tracker) = self
            .tracker_tiers
            .get_mut(next.0)
            .and_then(|tier| tier.get_mut(next.1))
        {
            tracker.schedule_immediate();
        }
        debug!(
            component = "tracker",
            operation = "private_failover",
            torrent = %self.info_hash_hex,
            from_tier = failed.0,
            to_tier = next.0,
            result = "switched",
            "private torrent advanced to the next tracker after failure"
        );
    }

    async fn disconnect_private_tracker_peers(&mut self) {
        self.known_tracker_peers.clear();
        let peers = std::mem::take(&mut self.peers);
        let mut tasks = futures::stream::FuturesUnordered::new();
        for (_, peer) in peers {
            peer.peer_control.cancel();
            tasks.push(peer.task);
        }
        self.join_v2_peer_tasks(tasks, "disconnect_private_tracker_peers")
            .await;
    }

    async fn announce_stopped(&mut self) {
        if !consume_v2_stopped_announce(&mut self.stopped_announced) {
            return;
        }
        let candidates = tracker_keys_for_stopped_announce(
            &self.tracker_tiers,
            self.meta.private,
            self.private_tracker_key,
        )
        .into_iter()
        .filter_map(|(tier_index, tracker_index)| {
            let tracker = self.tracker_tiers.get(tier_index)?.get(tracker_index)?;
            Some((
                tier_index,
                tracker_index,
                tracker.url.clone(),
                tracker.tracker_id.clone(),
            ))
        })
        .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }

        let context = self.tracker_announce_context().await;
        let mut pending = futures::stream::iter(candidates.into_iter().map(
            |(tier_index, tracker_index, url, tracker_id)| {
                let context = context.clone();
                async move {
                    let result = announce_tracker(
                        &context,
                        &url,
                        TrackerEvent::Stopped,
                        tracker_id.as_deref(),
                    )
                    .await
                    .map(|response| response.response);
                    (tier_index, tracker_index, url, result)
                }
            },
        ))
        .buffer_unordered(MAX_TRACKER_ANNOUNCES_IN_FLIGHT);
        let deadline = sleep(STOPPED_TRACKER_ANNOUNCE_DEADLINE);
        tokio::pin!(deadline);
        let mut results = Vec::new();
        loop {
            tokio::select! {
                result = pending.next() => {
                    let Some(result) = result else { break };
                    results.push(result);
                }
                _ = &mut deadline => {
                    warn!(
                        component = "tracker",
                        operation = "announce_stopped",
                        torrent = %self.info_hash_hex,
                        result = "deadline_exceeded",
                        deadline_secs = STOPPED_TRACKER_ANNOUNCE_DEADLINE.as_secs(),
                        completed = results.len(),
                        "pure-v2 stopped tracker announces exceeded aggregate deadline"
                    );
                    break;
                }
            }
        }

        let had_results = !results.is_empty();
        for (tier_index, tracker_index, url, result) in results {
            let Some(tracker) = self
                .tracker_tiers
                .get_mut(tier_index)
                .and_then(|tier| tier.get_mut(tracker_index))
            else {
                continue;
            };
            match result {
                Ok(response) => {
                    tracker.on_success_with_min_interval(&response, self.min_announce_interval);
                }
                Err(error) => {
                    warn!(
                        component = "tracker",
                        operation = "announce_stopped",
                        torrent = %self.info_hash_hex,
                        tracker = %url_log_target(&url),
                        result = "error",
                        error = %error,
                        "pure-v2 stopped tracker announce failed"
                    );
                    tracker.on_failure(error);
                }
            }
        }
        if had_results {
            let _ = self.persist_tracker_state().await;
        }
    }

    async fn announce_stopped_with_control_deadline(&mut self) {
        if timeout(
            STOPPED_TRACKER_CONTROL_ANNOUNCE_DEADLINE,
            self.announce_stopped(),
        )
        .await
        .is_err()
        {
            warn!(
                component = "tracker",
                operation = "announce_stopped",
                torrent = %self.info_hash_hex,
                result = "control_deadline_exceeded",
                deadline_ms = STOPPED_TRACKER_CONTROL_ANNOUNCE_DEADLINE.as_millis(),
                "pure-v2 stopped tracker announce exceeded the lifecycle control deadline"
            );
        }
    }

    async fn persist_tracker_state(&self) -> Result<(), String> {
        let (uploaded, downloaded) = {
            let registry = self.registry.read().await;
            registry
                .get(&self.info_hash_hex)
                .map(|entry| (entry.stats.uploaded, entry.stats.downloaded))
                .unwrap_or((0, 0))
        };
        let have = self.local_have.read().await.clone();
        let left = v2_db_i64(self.bytes_left(have.as_ref()));
        let now = Instant::now();
        let mut rows = Vec::new();
        let mut tracker_index = 0i64;
        let mut tracker_message = None;
        for (tier_index, tier) in self.tracker_tiers.iter().enumerate() {
            for tracker in tier {
                let failure_reason = v2_tracker_failure_reason(&tracker.status);
                let warning_message = v2_tracker_warning_message(&tracker.status);
                if tracker_message.is_none() {
                    tracker_message = failure_reason.clone().or_else(|| warning_message.clone());
                }
                rows.push(rt_db::TorrentTrackerRow {
                    info_hash: self.info_hash_hex.clone(),
                    tracker_index,
                    tier: i64::try_from(tier_index).unwrap_or(i64::MAX),
                    url: tracker.url.clone(),
                    tracker_id: tracker.tracker_id.clone(),
                    status: v2_tracker_status_label(&tracker.status).to_owned(),
                    last_announce_at: v2_instant_to_unix(tracker.last_announce, now),
                    next_announce_at: v2_instant_to_unix(tracker.next_announce, now),
                    last_success_at: v2_instant_to_unix(tracker.last_success, now),
                    failure_reason,
                    warning_message,
                    seeders: tracker.scrape_complete.map(i64::from),
                    leechers: tracker.scrape_incomplete.map(i64::from),
                    completed: tracker.scrape_downloaded.map(i64::from),
                    uploaded: v2_db_i64(uploaded),
                    downloaded: v2_db_i64(downloaded),
                    left_bytes: left,
                });
                tracker_index = tracker_index.saturating_add(1);
            }
        }
        let info_hash = self.info_hash_hex.clone();
        self.db
            .run("persist_v2_tracker_state", move |db| {
                rt_db::replace_torrent_trackers(db, &info_hash, &rows)
                    .map_err(|error| error.to_string())
            })
            .await?;
        if let Some(mut entry) = self.registry.write().await.get_mut(&self.info_hash_hex) {
            entry.tracker_message = tracker_message;
        }
        Ok(())
    }

    async fn resume_runtime(&mut self, _reply_requested: bool) -> Result<(), String> {
        let was_paused = self.paused;
        self.paused = false;
        self.tracker_event = TrackerEvent::Started;
        self.stopped_announced = false;
        self.schedule_trackers_now();
        let result = async {
            let state = self
                .registry
                .read()
                .await
                .get(&self.info_hash_hex)
                .map(|entry| entry.state)
                .ok_or_else(|| {
                    format!(
                        "torrent {} is missing from the registry",
                        self.info_hash_hex
                    )
                })?;
            if state != TorrentState::Checking {
                self.persist_runtime(Some(TorrentState::Checking), None)
                    .await?;
            }
            Ok::<(), String>(())
        }
        .await;
        if result.is_err() {
            self.paused = was_paused;
            self.tracker_event = TrackerEvent::Started;
            self.stopped_announced = was_paused;
            self.tracker_workers.cancel();
        }
        result
    }

    async fn recheck_and_set_state(&mut self) {
        // Peer contexts retain an immutable file-policy snapshot and may have
        // advertised pieces that the recheck invalidates.  Recheck from a
        // clean peer set so no connection can use stale availability.
        self.shutdown_peers().await;
        let new_have = self.recheck_files().await;
        self.report_v2_recheck(&new_have, "recheck").await;
        *self.local_have.write().await = Arc::new(new_have);
        self.notify_local_have_changed();
        let target = if self.paused {
            TorrentState::Paused
        } else if self.is_complete().await {
            TorrentState::Seeding
        } else {
            TorrentState::Downloading
        };
        if let Err(error) = self.persist_runtime(Some(target), None).await {
            warn!(
                component = "torrent_v2",
                operation = "recheck_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist pure-v2 recheck state"
            );
        } else if let Err(error) = self.persist_tracker_state().await {
            warn!(
                component = "torrent_v2",
                operation = "recheck_tracker_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist pure-v2 tracker state after recheck"
            );
        }
    }

    async fn recheck_files(&self) -> V2Bitmap {
        let files = self
            .meta
            .files
            .iter()
            .filter(|file| !file.pad)
            .map(|file| V2FileHash {
                file_index: file.index,
                path: file.path.clone(),
                length: file.length,
                pieces_root: file.pieces_root,
            })
            .collect::<Vec<_>>();
        let verifier = V2FileVerifier::new(&self.save_root, &self.storage, &files);
        let results = verifier.verify_all().await;
        let mut have = V2Bitmap::new(self.piece_map.piece_count as usize);
        // BEP 47 padding is synthetic zero content. Clients implementing the
        // extension need not materialize it, so a missing `.pad/*` path must
        // not make the torrent appear incomplete. Mark its address-space
        // pieces available without passing it through the file verifier.
        for file in self.meta.files.iter().filter(|file| file.pad) {
            if let Ok(ranges) = self.piece_map.piece_ranges_for_file_indices(&[file.index]) {
                for (first, last) in ranges {
                    for piece in first..last {
                        have.set(piece as usize, true);
                    }
                }
            }
        }
        for (file_index, result) in results {
            match result {
                VerifyResult::Valid => {
                    if let Ok(ranges) = self.piece_map.piece_ranges_for_file_indices(&[file_index])
                    {
                        for (first, last) in ranges {
                            for piece in first..last {
                                have.set(piece as usize, true);
                            }
                        }
                    }
                }
                VerifyResult::Invalid | VerifyResult::Missing { .. } => {}
            }
        }
        have
    }

    async fn is_complete(&self) -> bool {
        let have = self.local_have.read().await;
        self.bytes_left(have.as_ref()) == 0
    }

    fn bytes_left(&self, have: &V2Bitmap) -> u64 {
        (0..self.piece_map.piece_count)
            .filter(|piece| self.piece_is_wanted(*piece) && !have.get(*piece as usize))
            .filter_map(|piece| self.piece_map.piece_len(piece).ok())
            .map(u64::from)
            .sum()
    }

    async fn persist_runtime(
        &self,
        target_state: Option<TorrentState>,
        transfer: Option<(bool, u64)>,
    ) -> Result<(), String> {
        let have = self.local_have.read().await.clone();
        let mut amount_left = self.bytes_left(have.as_ref());
        let held = self
            .completion_held
            .load(std::sync::atomic::Ordering::Relaxed)
            && target_state != Some(TorrentState::Seeding);
        let finalizing = held && amount_left == 0;
        if finalizing {
            // Complete on disk but not yet durable: report one byte remaining.
            amount_left = 1;
        }
        let (previous, row) = {
            let mut registry = self.registry.write().await;
            let mut entry = registry.get_mut(&self.info_hash_hex).ok_or_else(|| {
                format!(
                    "torrent {} is missing from the registry",
                    self.info_hash_hex
                )
            })?;
            let previous = entry.clone();
            if let Some((upload, bytes)) = transfer {
                if upload {
                    entry.stats.add_upload(bytes);
                } else {
                    entry.stats.add_download(bytes);
                }
            }
            if let Some(state) = target_state {
                entry.transition(state).map_err(|error| error.to_string())?;
            }
            entry.total_length = self.meta.total_length();
            entry.amount_left = amount_left;
            entry.finalizing = finalizing;
            if entry.state == TorrentState::Seeding && entry.completed_at.is_none() {
                entry.completed_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
                entry.amount_left = 0;
                entry.finalizing = false;
            }
            let row = crate::engine::row_from_v2_meta(&entry, &self.meta);
            (previous, row)
        };
        let persistence = self
            .db
            .run_batched("persist_v2_torrent_runtime", move |tx| {
                let updated =
                    rt_db::update_runtime_in_tx(tx, &row).map_err(|error| error.to_string())?;
                if !updated {
                    return Err(format!(
                        "torrent {} is missing from the database",
                        row.info_hash
                    ));
                }
                Ok(())
            })
            .await;
        if let Err(error) = persistence {
            if let Some(mut entry) = self.registry.write().await.get_mut(&self.info_hash_hex) {
                restore_entry_runtime(&mut entry, &previous);
            }
            return Err(error);
        }
        Ok(())
    }

    async fn handle_peer_event(&mut self, event: V2PeerEvent) {
        match event {
            V2PeerEvent::Downloaded { peer, bytes } => {
                let complete = self.is_complete().await;
                let release = complete && self.completion_may_release().await;
                let target = release.then_some(TorrentState::Seeding);
                if let Err(error) = self.persist_runtime(target, Some((false, bytes))).await {
                    debug!(
                        component = "torrent_v2",
                        operation = "persist_download",
                        torrent = %self.info_hash_hex,
                        peer = %peer,
                        result = "error",
                        error = %error,
                        "failed to persist pure-v2 download progress"
                    );
                }
            }
            V2PeerEvent::Uploaded { peer, bytes } => {
                if let Err(error) = self.persist_runtime(None, Some((true, bytes))).await {
                    debug!(
                        component = "torrent_v2",
                        operation = "persist_upload",
                        torrent = %self.info_hash_hex,
                        peer = %peer,
                        result = "error",
                        error = %error,
                        "failed to persist pure-v2 upload progress"
                    );
                }
            }
            V2PeerEvent::Disconnected { peer } => {
                self.peers.remove(&peer);
            }
            V2PeerEvent::InterestChanged => {
                if !self.paused {
                    self.run_choker().await;
                }
            }
        }
    }

    /// Apply the same mode-aware regular and optimistic upload slots used by
    /// the v1 actor. Peer loops own their sockets; this task publishes only
    /// the selected connection IDs through a watch channel.
    async fn run_choker(&mut self) {
        let mode = if self.is_complete().await {
            ChokeMode::Seeding
        } else {
            ChokeMode::Downloading
        };
        let snapshots = self
            .peers
            .iter()
            .filter_map(|(_, handle)| {
                let state = handle.state.lock().ok()?;
                Some(PeerSnapshot {
                    id: handle.choker_id,
                    interested: state.interested,
                    upload_rate: state.rate(true) as f64,
                    download_rate: state.rate(false) as f64,
                    current_choke: if state.upload_choked {
                        ChokeState::Choked
                    } else {
                        ChokeState::Unchoked
                    },
                })
            })
            .collect::<Vec<_>>();
        let decisions = self.choker.run_with_mode(&snapshots, mode);
        let selected = self
            .peers
            .iter()
            .filter_map(|(_, handle)| {
                (decisions.get(&handle.choker_id) == Some(&ChokeDecision::Unchoke))
                    .then_some(handle.choker_id)
            })
            .collect::<HashSet<_>>();
        let previous = {
            let current = self.upload_unchoked.borrow();
            Arc::clone(&*current)
        };
        if previous.as_ref() != &selected {
            self.upload_unchoked.send_replace(Arc::new(selected));
        }
    }

    fn peer_capacity(&self) -> usize {
        self.torrent_max_peers
            .map(|limit| limit.min(self.max_peers))
            .unwrap_or(self.max_peers)
    }

    fn peer_context(&self, peer: SocketAddr, choker_id: PeerId) -> V2PeerContext {
        let peer_control = RateLimitCancellation::default();
        let peer_generation = peer_control.generation();
        V2PeerContext {
            info_hash: self.meta.info_hash_v2[..20]
                .try_into()
                .expect("v2 info hash truncation is 20 bytes"),
            peer,
            choker_id,
            meta: Arc::clone(&self.meta),
            piece_map: Arc::clone(&self.piece_map),
            save_root: self.save_root.clone(),
            storage: self.storage.clone(),
            resources: self.resources.clone(),
            network_budget: self.network_budget.clone(),
            peer_control,
            peer_generation,
            upload_control: RateLimitCancellation::default(),
            local_have: Arc::clone(&self.local_have),
            have_updates: self.have_updates.clone(),
            upload_unchoked: self.upload_unchoked.subscribe(),
            file_policy: Arc::clone(&self.file_policy),
            assembly_cap_bytes: self.assembly_cap_bytes,
            events: self.events_tx.clone(),
            proof_cache: Arc::clone(&self.proof_cache),
        }
    }

    async fn connect_peers(&mut self, peers: Vec<SocketAddr>, source: PeerSource) {
        for peer in peers {
            if self.peers.len() >= self.peer_capacity() {
                break;
            }
            if let Err(error) = self.egress_policy.validate_peer_addr(peer) {
                debug!(
                    component = "torrent_v2",
                    operation = "connect_peer",
                    peer = %peer,
                    result = "rejected",
                    reason = "egress_address_policy",
                    error = %error,
                    "skipping outgoing peer denied by address policy"
                );
                continue;
            }
            if self.peers.contains_key(&peer)
                || self.registry.read().await.is_peer_banned(peer)
                || !private_peer_source_allowed(self.meta.private, &self.known_tracker_peers, peer)
            {
                continue;
            }
            let Ok(peer_permit) = self.network_budget.try_acquire_peer() else {
                break;
            };
            // V2PeerState retains remote_have and the protocol loop retains
            // announced_have, so charge both packed maps before spawning it.
            let bitmap_bytes = V2Bitmap::memory_bytes_for_len(self.piece_map.piece_count as usize)
                .saturating_mul(2);
            let Some(bitmap_memory_lease) = self
                .resources
                .try_acquire(MemoryClass::PeerBuffer, bitmap_bytes)
            else {
                drop(peer_permit);
                break;
            };
            let state = Arc::new(Mutex::new(V2PeerState::new(
                self.piece_map.piece_count as usize,
            )));
            let choker_id = PeerId::new();
            let context = self.peer_context(peer, choker_id);
            let peer_control = context.peer_control.clone();
            let event_peer = peer;
            let task_state = Arc::clone(&state);
            let transport_policy = outgoing_transport_policy_for_peer(
                outgoing_transport_policy_configured(),
                source,
                self.meta.private,
            );
            let task = tokio::spawn(async move {
                let _peer_permit = peer_permit;
                if let Err(error) =
                    run_outgoing_v2_peer(context.clone(), peer, task_state, transport_policy).await
                {
                    debug!(
                        component = "torrent_v2",
                        operation = "connect_peer",
                        peer = %peer,
                        result = "ended",
                        error = %error,
                        "pure-v2 peer connection ended"
                    );
                }
                let _ = context
                    .events
                    .send(V2PeerEvent::Disconnected { peer: event_peer })
                    .await;
            });
            self.peers.insert(
                peer,
                V2PeerHandle {
                    state,
                    choker_id,
                    peer_control,
                    task,
                    _bitmap_memory_lease: bitmap_memory_lease,
                },
            );
        }
    }

    async fn connect_priority_peers(&mut self, peers: Vec<SocketAddr>) {
        let mut allowed_peers = Vec::with_capacity(peers.len());
        for peer in peers {
            if let Err(error) = self.egress_policy.validate_peer_addr(peer) {
                debug!(
                    component = "torrent_v2",
                    operation = "connect_priority",
                    peer = %peer,
                    result = "rejected",
                    reason = "egress_address_policy",
                    error = %error,
                    "skipping priority peer denied by address policy"
                );
                continue;
            }
            allowed_peers.push(peer);
        }
        let preferred = allowed_peers
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        for peer in allowed_peers {
            if self.peers.len() >= self.peer_capacity() {
                let victim = self
                    .peers
                    .iter()
                    .find(|(addr, state)| {
                        !preferred.contains(addr)
                            && state.state.lock().is_ok_and(|state| state.outstanding == 0)
                    })
                    .map(|(addr, _)| *addr);
                if let Some(victim) = victim {
                    self.remove_peer(victim).await;
                }
            }
            if self.peers.len() >= self.peer_capacity() {
                break;
            }
            self.connect_peers(vec![peer], PeerSource::Manual).await;
        }
    }

    fn remember_tracker_peers(&mut self, peers: &[SocketAddr]) {
        let cap = tracker_peer_cache_cap(self.max_peers);
        let mut dropped = 0u64;
        for &peer in peers {
            if self.known_tracker_peers.contains(&peer) {
                continue;
            }
            if self._tracker_peer_cache_memory_lease.is_none()
                || self.known_tracker_peers.len() >= cap
            {
                dropped = dropped.saturating_add(1);
                continue;
            }
            // The shared cache constructor reserves the complete bounded
            // capacity before any tracker response is accepted, so this
            // insert cannot grow an uncharged HashSet allocation.
            self.known_tracker_peers.insert(peer);
        }
        self.tracker_peer_cache_drops = self.tracker_peer_cache_drops.saturating_add(dropped);
    }

    async fn accept_peer(
        &mut self,
        stream: TcpStream,
        peer: SocketAddr,
        handshake: Handshake,
        peer_permit: OwnedSemaphorePermit,
    ) {
        if handshake.info_hash != self.meta.info_hash_v2[..20]
            || !handshake.reserved.supports_v2()
            || handshake.peer_id == crate::peer_id::our_peer_id()
            || self.peers.len() >= self.peer_capacity()
            || self.peers.contains_key(&peer)
            || self.registry.read().await.is_peer_banned(peer)
            || !private_peer_source_allowed(self.meta.private, &self.known_tracker_peers, peer)
        {
            drop(peer_permit);
            return;
        }
        // V2PeerState retains remote_have and the protocol loop retains
        // announced_have, so charge both packed maps before spawning it.
        let bitmap_bytes =
            V2Bitmap::memory_bytes_for_len(self.piece_map.piece_count as usize).saturating_mul(2);
        let Some(bitmap_memory_lease) = self
            .resources
            .try_acquire(MemoryClass::PeerBuffer, bitmap_bytes)
        else {
            drop(peer_permit);
            return;
        };
        let state = Arc::new(Mutex::new(V2PeerState::new(
            self.piece_map.piece_count as usize,
        )));
        let choker_id = PeerId::new();
        let context = self.peer_context(peer, choker_id);
        let peer_control = context.peer_control.clone();
        let task_state = Arc::clone(&state);
        let task = tokio::spawn(async move {
            let _peer_permit = peer_permit;
            if let Err(error) =
                run_incoming_v2_peer(context.clone(), stream, peer, handshake, task_state).await
            {
                debug!(
                    component = "torrent_v2",
                    operation = "accept_peer",
                    peer = %peer,
                    result = "ended",
                    error = %error,
                    "pure-v2 inbound peer ended"
                );
            }
            let _ = context
                .events
                .send(V2PeerEvent::Disconnected { peer })
                .await;
        });
        self.peers.insert(
            peer,
            V2PeerHandle {
                state,
                choker_id,
                peer_control,
                task,
                _bitmap_memory_lease: bitmap_memory_lease,
            },
        );
    }

    async fn accept_utp_peer(
        &mut self,
        stream: UtpStream,
        peer: SocketAddr,
        handshake: Handshake,
        peer_permit: OwnedSemaphorePermit,
    ) {
        if handshake.info_hash != self.meta.info_hash_v2[..20]
            || !handshake.reserved.supports_v2()
            || handshake.peer_id == crate::peer_id::our_peer_id()
            || self.peers.len() >= self.peer_capacity()
            || self.peers.contains_key(&peer)
            || self.registry.read().await.is_peer_banned(peer)
            || !private_peer_source_allowed(self.meta.private, &self.known_tracker_peers, peer)
        {
            drop(peer_permit);
            return;
        }
        // V2PeerState retains remote_have and the protocol loop retains
        // announced_have, so charge both packed maps before spawning it.
        let bitmap_bytes =
            V2Bitmap::memory_bytes_for_len(self.piece_map.piece_count as usize).saturating_mul(2);
        let Some(bitmap_memory_lease) = self
            .resources
            .try_acquire(MemoryClass::PeerBuffer, bitmap_bytes)
        else {
            drop(peer_permit);
            return;
        };
        let state = Arc::new(Mutex::new(V2PeerState::new(
            self.piece_map.piece_count as usize,
        )));
        let choker_id = PeerId::new();
        let context = self.peer_context(peer, choker_id);
        let peer_control = context.peer_control.clone();
        let task_state = Arc::clone(&state);
        let task = tokio::spawn(async move {
            let _peer_permit = peer_permit;
            if let Err(error) =
                run_incoming_v2_utp_peer(context.clone(), stream, peer, handshake, task_state).await
            {
                debug!(
                    component = "torrent_v2",
                    operation = "accept_utp_peer",
                    peer = %peer,
                    result = "ended",
                    error = %error,
                    "pure-v2 inbound uTP peer ended"
                );
            }
            let _ = context
                .events
                .send(V2PeerEvent::Disconnected { peer })
                .await;
        });
        self.peers.insert(
            peer,
            V2PeerHandle {
                state,
                choker_id,
                peer_control,
                task,
                _bitmap_memory_lease: bitmap_memory_lease,
            },
        );
    }

    async fn remove_peer(&mut self, peer: SocketAddr) {
        let Some(handle) = self.peers.remove(&peer) else {
            return;
        };
        handle.peer_control.cancel();
        let mut tasks = futures::stream::FuturesUnordered::new();
        tasks.push(handle.task);
        self.join_v2_peer_tasks(tasks, "remove_peer").await;
    }

    async fn shutdown_peers(&mut self) {
        let peers = std::mem::take(&mut self.peers);
        let mut tasks = futures::stream::FuturesUnordered::new();
        for (_, handle) in peers {
            handle.peer_control.cancel();
            tasks.push(handle.task);
        }
        self.join_v2_peer_tasks(tasks, "shutdown_peers").await;
    }

    async fn join_v2_peer_tasks(
        &mut self,
        mut tasks: futures::stream::FuturesUnordered<tokio::task::JoinHandle<()>>,
        operation: &'static str,
    ) {
        while !tasks.is_empty() {
            let (joined, event) = tokio::select! {
                joined = tasks.next() => (Some(joined), None),
                event = self.events_rx.recv() => (None, event),
            };
            if let Some(Some(Err(error))) = joined {
                if error.is_panic() {
                    crate::log_task_join_error("torrent_v2", operation, "peer task", &error);
                }
            }
            if let Some(event) = event {
                match event {
                    V2PeerEvent::Downloaded { .. } | V2PeerEvent::Uploaded { .. } => {
                        self.handle_peer_event(event).await;
                    }
                    V2PeerEvent::InterestChanged | V2PeerEvent::Disconnected { .. } => {}
                }
            }
        }
    }

    fn peer_snapshots_unbounded(&self) -> Vec<EnginePeerSnapshot> {
        self.peers
            .iter()
            .filter_map(|(addr, peer)| {
                let state = peer.state.lock().ok()?;
                let pieces = state.remote_have.count_ones();
                let pieces_total = state.remote_have.len();
                Some(EnginePeerSnapshot {
                    addr: *addr,
                    client: "BEP 52 peer".to_owned(),
                    choked: state.choked,
                    upload_choked: state.upload_choked,
                    interested: state.interested,
                    pieces,
                    pieces_total,
                    progress: if pieces_total == 0 {
                        0.0
                    } else {
                        pieces as f64 / pieces_total as f64
                    },
                    download_rate: state.rate(false),
                    upload_rate: state.rate(true),
                    downloaded: state.downloaded,
                    uploaded: state.uploaded,
                })
            })
            .collect()
    }

    fn runtime_stats(&self) -> TorrentRuntimeStats {
        let mut outstanding = 0u64;
        let mut download_rate = 0i64;
        let mut upload_rate = 0i64;
        let mut piece_assembly_buffers = 0u64;
        let mut piece_assembly_bytes = 0u64;
        for peer in self.peers.values() {
            if let Ok(state) = peer.state.lock() {
                outstanding = outstanding.saturating_add(state.outstanding as u64);
                download_rate = download_rate.saturating_add(state.rate(false));
                upload_rate = upload_rate.saturating_add(state.rate(true));
                piece_assembly_buffers =
                    piece_assembly_buffers.saturating_add(state.assembly_buffers);
                piece_assembly_bytes = piece_assembly_bytes.saturating_add(state.assembly_bytes);
            }
        }
        TorrentRuntimeStats {
            connected_peers: self.peers.len() as u64,
            outstanding_requests: outstanding,
            download_rate,
            upload_rate,
            piece_assembly_buffers,
            piece_assembly_bytes,
            tracker_peer_cache_entries: self.known_tracker_peers.len() as u64,
            tracker_peer_cache_drops: self.tracker_peer_cache_drops,
            tracker_peer_cache_bytes: self
                ._tracker_peer_cache_memory_lease
                .as_ref()
                .map_or(0, MemoryLease::bytes),
            storage: self.storage.stats(),
            ..TorrentRuntimeStats::default()
        }
    }

    fn reject_pending_commands(&mut self) {
        while let Ok(command) = self.cmd_rx.try_recv() {
            reject_v2_command(command);
        }
    }
}

fn restore_entry_runtime(entry: &mut TorrentEntry, previous: &TorrentEntry) {
    entry.state = previous.state;
    entry.total_length = previous.total_length;
    entry.amount_left = previous.amount_left;
    entry.stats = previous.stats.clone();
    entry.completed_at = previous.completed_at;
    entry.error_message = previous.error_message.clone();
    entry.tracker_message = previous.tracker_message.clone();
}

fn reserve_v2_tracker_state_memory(
    resources: &ResourceGovernor,
    tiers: &[Vec<TrackerState>],
) -> Option<MemoryLease> {
    let bytes = tiers
        .iter()
        .map(|tier| {
            std::mem::size_of::<Vec<TrackerState>>()
                .saturating_add(
                    tier.capacity()
                        .saturating_mul(std::mem::size_of::<TrackerState>()),
                )
                .saturating_add(
                    tier.iter()
                        .map(|tracker| tracker.url.capacity().saturating_add(32 * 1024))
                        .sum::<usize>(),
                )
        })
        .sum::<usize>();
    if bytes == 0 {
        return None;
    }
    resources.try_acquire(
        MemoryClass::Metadata,
        u64::try_from(bytes).unwrap_or(u64::MAX),
    )
}

fn v2_db_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn v2_tracker_event_after_success(current: TrackerEvent, sent: TrackerEvent) -> TrackerEvent {
    if current == sent && matches!(sent, TrackerEvent::Started | TrackerEvent::Completed) {
        TrackerEvent::Empty
    } else {
        current
    }
}

fn consume_v2_stopped_announce(stopped_announced: &mut bool) -> bool {
    if *stopped_announced {
        false
    } else {
        *stopped_announced = true;
        true
    }
}

fn v2_tracker_status_label(status: &TrackerStatus) -> &'static str {
    match status {
        TrackerStatus::NeverAnnounced => "never_announced",
        TrackerStatus::Announcing => "announcing",
        TrackerStatus::Working => "working",
        TrackerStatus::Warning(_) => "warning",
        TrackerStatus::Error(_) => "error",
        TrackerStatus::Disabled => "disabled",
    }
}

fn v2_tracker_failure_reason(status: &TrackerStatus) -> Option<String> {
    match status {
        TrackerStatus::Error(error) => Some(error.to_string()),
        _ => None,
    }
}

fn v2_tracker_warning_message(status: &TrackerStatus) -> Option<String> {
    match status {
        TrackerStatus::Warning(message) => Some(message.clone()),
        _ => None,
    }
}

fn v2_instant_to_unix(instant: Option<Instant>, now: Instant) -> Option<i64> {
    let current = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    instant.map(|instant| {
        if instant >= now {
            current.saturating_add(instant.duration_since(now).as_secs() as i64)
        } else {
            current.saturating_sub(now.duration_since(instant).as_secs() as i64)
        }
    })
}

fn reject_v2_command(command: TorrentCmd) {
    const ERROR: &str = "torrent task is shutting down";
    match command {
        TorrentCmd::Pause { reply } | TorrentCmd::Resume { reply } => {
            if let Some(reply) = reply {
                let _ = reply.send(Err(ERROR.to_owned()));
            }
        }
        TorrentCmd::ReloadFilePolicy { reply } | TorrentCmd::UpdateLimits { reply, .. } => {
            if let Some(reply) = reply {
                let _ = reply.send(Err(ERROR.to_owned()));
            }
        }
        TorrentCmd::UpdateTrackers { reply, .. } => {
            if let Some(reply) = reply {
                let _ = reply.send(Err(ERROR.to_owned()));
            }
        }
        TorrentCmd::QuiesceForStorageMove { reply } => {
            let _ = reply.send(Err(ERROR.to_owned()));
        }
        TorrentCmd::ResumeAfterStorageMove { reply, .. } => {
            let _ = reply.send(Err(ERROR.to_owned()));
        }
        TorrentCmd::GetPeers { reply } => {
            let _ = reply.send(Vec::new());
        }
        TorrentCmd::GetPeerSnapshotCount { reply } => {
            let _ = reply.send(0);
        }
        TorrentCmd::GetPeerSnapshots { reply, .. } => {
            let _ = reply.send(Err(ERROR.to_owned()));
        }
        TorrentCmd::GetWebseeds { reply } => {
            let _ = reply.send(Vec::new());
        }
        TorrentCmd::GetRuntimeStats { reply } => {
            let _ = reply.send(TorrentRuntimeStats::default());
        }
        TorrentCmd::AcceptPeer { .. }
        | TorrentCmd::AcceptUtpPeer { .. }
        | TorrentCmd::Shutdown
        | TorrentCmd::Recheck { .. }
        | TorrentCmd::CancelJob { .. }
        | TorrentCmd::Reannounce
        | TorrentCmd::NewPeers(_)
        | TorrentCmd::PriorityPeers(_)
        | TorrentCmd::UpdatePeerExchange(_)
        | TorrentCmd::BanPeer(_)
        | TorrentCmd::EvictBannedPeers => {}
    }
}

async fn run_outgoing_v2_peer(
    context: V2PeerContext,
    addr: SocketAddr,
    state: Arc<Mutex<V2PeerState>>,
    transport_policy: OutgoingTransportPolicy,
) -> anyhow::Result<()> {
    if matches!(
        transport_policy,
        OutgoingTransportPolicy::PreferUtp | OutgoingTransportPolicy::UtpOnly
    ) {
        match run_outgoing_v2_utp_peer(context.clone(), addr, Arc::clone(&state)).await {
            Ok(()) => return Ok(()),
            Err(error) if transport_policy == OutgoingTransportPolicy::PreferUtp => {
                debug!(
                    component = "torrent_v2",
                    operation = "connect_utp",
                    peer = %addr,
                    result = "fallback",
                    error = %error,
                    "pure-v2 uTP setup failed; falling back to TCP"
                );
            }
            Err(error) => return Err(error),
        }
    }
    let stream = timeout(V2_HANDSHAKE_TIMEOUT, TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;
    let mut framed = Framed::with_capacity(
        stream,
        PeerCodec::with_resources(context.resources.clone()),
        1,
    );
    let our_handshake = Handshake {
        info_hash: context.info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_v2_support(),
    };
    write_handshake(&mut framed, &our_handshake).await?;
    let remote = read_handshake(&mut framed).await?;
    validate_remote_v2_handshake(&remote, context.info_hash, addr)?;
    run_v2_peer_protocol(
        context,
        addr,
        PeerIo::Tcp(framed),
        state,
        remote.reserved.supports_extension_protocol(),
        remote.reserved.supports_fast_extension(),
    )
    .await
}

async fn run_outgoing_v2_utp_peer(
    context: V2PeerContext,
    addr: SocketAddr,
    state: Arc<Mutex<V2PeerState>>,
) -> anyhow::Result<()> {
    let mut stream = timeout(
        V2_HANDSHAKE_TIMEOUT,
        context.network_budget.utp_runtime().connect(addr),
    )
    .await??;
    let our_handshake = Handshake {
        info_hash: context.info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_v2_support(),
    };
    timeout(
        V2_SOCKET_WRITE_TIMEOUT,
        stream.write_all(&our_handshake.encode()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("v2 uTP handshake write timed out"))??;
    let mut bytes = [0u8; rt_peer_wire::handshake::HANDSHAKE_LEN];
    timeout(V2_HANDSHAKE_TIMEOUT, stream.read_exact(&mut bytes)).await??;
    let remote = Handshake::parse(&bytes)?;
    validate_remote_v2_handshake(&remote, context.info_hash, addr)?;
    run_v2_peer_protocol(
        context.clone(),
        addr,
        PeerIo::Utp(Box::new(UtpPeerIo::new(stream, context.resources.clone()))),
        state,
        remote.reserved.supports_extension_protocol(),
        remote.reserved.supports_fast_extension(),
    )
    .await
}

async fn run_incoming_v2_peer(
    context: V2PeerContext,
    stream: TcpStream,
    addr: SocketAddr,
    remote: Handshake,
    state: Arc<Mutex<V2PeerState>>,
) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    let mut framed = Framed::with_capacity(
        stream,
        PeerCodec::with_resources(context.resources.clone()),
        1,
    );
    let our_handshake = Handshake {
        info_hash: context.info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_v2_support(),
    };
    write_handshake(&mut framed, &our_handshake).await?;
    validate_remote_v2_handshake(&remote, context.info_hash, addr)?;
    run_v2_peer_protocol(
        context,
        addr,
        PeerIo::Tcp(framed),
        state,
        remote.reserved.supports_extension_protocol(),
        remote.reserved.supports_fast_extension(),
    )
    .await
}

async fn run_incoming_v2_utp_peer(
    context: V2PeerContext,
    mut stream: UtpStream,
    addr: SocketAddr,
    remote: Handshake,
    state: Arc<Mutex<V2PeerState>>,
) -> anyhow::Result<()> {
    validate_remote_v2_handshake(&remote, context.info_hash, addr)?;
    let our_handshake = Handshake {
        info_hash: context.info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_v2_support(),
    };
    timeout(
        V2_SOCKET_WRITE_TIMEOUT,
        stream.write_all(&our_handshake.encode()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("v2 uTP handshake write timed out"))??;
    run_v2_peer_protocol(
        context.clone(),
        addr,
        PeerIo::Utp(Box::new(UtpPeerIo::new(stream, context.resources.clone()))),
        state,
        remote.reserved.supports_extension_protocol(),
        remote.reserved.supports_fast_extension(),
    )
    .await
}

async fn write_handshake(
    framed: &mut Framed<TcpStream, PeerCodec>,
    handshake: &Handshake,
) -> anyhow::Result<()> {
    timeout(
        V2_SOCKET_WRITE_TIMEOUT,
        framed.get_mut().write_all(&handshake.encode()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("v2 peer handshake write timed out"))??;
    Ok(())
}

async fn read_handshake(framed: &mut Framed<TcpStream, PeerCodec>) -> anyhow::Result<Handshake> {
    let mut bytes = [0u8; rt_peer_wire::handshake::HANDSHAKE_LEN];
    timeout(
        V2_HANDSHAKE_TIMEOUT,
        framed.get_mut().read_exact(&mut bytes),
    )
    .await??;
    Handshake::parse(&bytes).map_err(Into::into)
}

fn validate_remote_v2_handshake(
    handshake: &Handshake,
    info_hash: [u8; 20],
    addr: SocketAddr,
) -> anyhow::Result<()> {
    if handshake.info_hash != info_hash {
        anyhow::bail!("v2 peer info_hash mismatch from {addr}");
    }
    if !handshake.reserved.supports_v2() {
        anyhow::bail!("peer {addr} does not advertise BEP 52 support");
    }
    if handshake.peer_id == crate::peer_id::our_peer_id() {
        anyhow::bail!("v2 peer handshake identifies this client");
    }
    Ok(())
}

/// Update the peer's local choke state and return the corresponding wire
/// transition only when the selected slot set actually changed.
fn set_v2_upload_unchoked(state: &Arc<Mutex<V2PeerState>>, unchoked: bool) -> Option<Message> {
    let mut peer = state.lock().ok()?;
    let currently_unchoked = !peer.upload_choked;
    if currently_unchoked == unchoked {
        return None;
    }
    peer.upload_choked = !unchoked;
    Some(if unchoked {
        Message::Unchoke
    } else {
        Message::Choke
    })
}

fn v2_upload_slot_selected(context: &V2PeerContext) -> bool {
    let selected = context.upload_unchoked.borrow();
    v2_slot_selected(&**selected, context.choker_id)
}

fn v2_slot_selected(selected: &HashSet<PeerId>, peer: PeerId) -> bool {
    selected.contains(&peer)
}

async fn run_v2_peer_protocol(
    context: V2PeerContext,
    addr: SocketAddr,
    mut framed: PeerIo,
    state: Arc<Mutex<V2PeerState>>,
    remote_supports_extension: bool,
    remote_supports_fast: bool,
) -> anyhow::Result<()> {
    let _upload_control_guard = V2UploadControlGuard(context.upload_control.clone());
    let shutdown_generation = context.peer_generation;
    if remote_supports_extension {
        if !send_v2_peer_message(
            &context,
            &mut framed,
            Message::Extended {
                ext_id: EXT_HANDSHAKE_ID,
                payload: ExtensionHandshake::new(None).encode(),
            },
        )
        .await?
        {
            return Ok(());
        }
    }
    let mut have_updates = context.have_updates.subscribe();
    let mut upload_unchoked = context.upload_unchoked.clone();
    // Keep the initial snapshot alive only until the first announcement has
    // been materialized. Retaining it for the whole peer lifetime would make
    // every later local-have replacement preserve an obsolete bitmap through
    // this peer's Arc reference.
    let mut announced_have = {
        let initial_have = context.local_have.read().await.clone();
        if !send_v2_have_bitmap(&context, &mut framed, &initial_have, remote_supports_fast).await? {
            return Ok(());
        }
        (*initial_have).clone()
    };

    let mut outstanding = HashMap::<(u32, u32), V2OutstandingRequest>::new();
    let mut assemblies = HashMap::<u32, V2PieceAssembly>::new();
    let mut transfer_jobs = futures::stream::FuturesUnordered::<V2TransferTask>::new();
    let mut pending_block_completions = HashMap::<(u32, u32), u64>::new();
    let mut pending_piece_completions = HashMap::<u32, u64>::new();
    let mut upload_reads = futures::stream::FuturesUnordered::<V2UploadReadTask>::new();
    let mut pending_upload_requests = VecDeque::new();
    let mut remote_availability_known = false;
    let mut local_interested = false;
    update_v2_interest(
        &context,
        &mut framed,
        &state,
        &mut local_interested,
        remote_availability_known,
    )
    .await?;
    let mut keepalive = interval(V2_KEEPALIVE_INTERVAL);
    let mut memory_retry = interval(Duration::from_secs(1));
    let mut last_activity = Instant::now();

    let protocol_result = async {
        loop {
        tokio::select! {
            biased;
            _ = context.peer_control.wait_for_change(shutdown_generation) => {
                return Ok(());
            }
            changed = upload_unchoked.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                context.upload_control.cancel();
                let selected = {
                    let current = upload_unchoked.borrow();
                    v2_slot_selected(&**current, context.choker_id)
                };
                if let Some(message) = set_v2_upload_unchoked(&state, selected) {
                    if !send_v2_peer_message(&context, &mut framed, message).await? {
                        return Ok(());
                    }
                }
                if selected {
                    start_v2_upload_reads(
                        &mut upload_reads,
                        &mut pending_upload_requests,
                        &context,
                        &state,
                    );
                } else {
                    while let Some(request) = pending_upload_requests.pop_front() {
                        reject_v2_request(
                            &context,
                            &mut framed,
                            remote_supports_fast,
                            request.piece,
                            request.begin,
                            request.length,
                        )
                        .await?;
                    }
                }
            }
            changed = have_updates.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                let current_have = context.local_have.read().await.clone();
                for piece_index in 0..current_have.len() {
                    if current_have.get(piece_index) && !announced_have.get(piece_index) {
                        let piece = u32::try_from(piece_index)
                            .map_err(|_| anyhow::anyhow!("v2 piece index exceeds wire range"))?;
                        if !send_v2_peer_message(
                            &context,
                            &mut framed,
                            Message::Have(piece),
                        )
                        .await?
                        {
                            return Ok(());
                        }
                    }
                    announced_have.set(piece_index, current_have.get(piece_index));
                }
            }
            message = framed.next() => {
                let message = message?;
                let Some(message) = message else {
                    return Ok(());
                };
                last_activity = Instant::now();
                match message {
                    Message::Bitfield(bits) => {
                        if let Ok(mut peer) = state.lock() {
                            Arc::make_mut(&mut peer.remote_have)
                                .copy_from_bitfield(&bits)
                                .map_err(anyhow::Error::msg)?;
                        }
                        remote_availability_known = true;
                        update_v2_interest(
                            &context,
                            &mut framed,
                            &state,
                            &mut local_interested,
                            remote_availability_known,
                        ).await?;
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    Message::Have(piece) => {
                        if piece >= context.piece_map.piece_count {
                            continue;
                        }
                        if let Ok(mut peer) = state.lock() {
                            Arc::make_mut(&mut peer.remote_have).set(piece as usize, true);
                        }
                        remote_availability_known = true;
                        update_v2_interest(
                            &context,
                            &mut framed,
                            &state,
                            &mut local_interested,
                            remote_availability_known,
                        ).await?;
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    Message::HaveAll => {
                        if let Ok(mut peer) = state.lock() {
                            Arc::make_mut(&mut peer.remote_have).fill(true);
                        }
                        remote_availability_known = true;
                        update_v2_interest(
                            &context,
                            &mut framed,
                            &state,
                            &mut local_interested,
                            remote_availability_known,
                        ).await?;
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    Message::HaveNone => {
                        if let Ok(mut peer) = state.lock() {
                            Arc::make_mut(&mut peer.remote_have).fill(false);
                        }
                        remote_availability_known = true;
                        update_v2_interest(
                            &context,
                            &mut framed,
                            &state,
                            &mut local_interested,
                            remote_availability_known,
                        ).await?;
                    }
                    Message::Unchoke => {
                        if let Ok(mut peer) = state.lock() {
                            peer.choked = false;
                        }
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    Message::Choke => {
                        apply_v2_remote_choke(
                            remote_supports_fast,
                            &mut outstanding,
                            &mut assemblies,
                        );
                        if let Ok(mut peer) = state.lock() {
                            peer.choked = true;
                            peer.outstanding = outstanding.len();
                            update_v2_assembly_stats(
                                &mut peer,
                                &assemblies,
                                &pending_block_completions,
                                &pending_piece_completions,
                            );
                        }
                    }
                    Message::Interested => {
                        let changed = if let Ok(mut peer) = state.lock() {
                            let changed = !peer.interested;
                            peer.interested = true;
                            changed
                        } else {
                            false
                        };
                        if changed {
                            let _ = context
                                .events
                                .send(V2PeerEvent::InterestChanged)
                                .await;
                        }
                    }
                    Message::NotInterested => {
                        let changed = if let Ok(mut peer) = state.lock() {
                            let changed = peer.interested;
                            peer.interested = false;
                            changed
                        } else {
                            false
                        };
                        if changed {
                            let _ = context
                                .events
                                .send(V2PeerEvent::InterestChanged)
                                .await;
                        }
                    }
                    Message::Request { piece, begin, length } => {
                        let request = V2UploadRequest {
                            piece,
                            begin,
                            length,
                        };
                        if !v2_upload_can_serve(&context, &state) {
                            reject_v2_request(
                                &context,
                                &mut framed,
                                remote_supports_fast,
                                piece,
                                begin,
                                length,
                            )
                            .await?;
                        } else if pending_upload_requests.len() + upload_reads.len()
                            >= V2_MAX_UPLOAD_WORK
                        {
                            if remote_supports_fast {
                                reject_v2_request(
                                    &context,
                                    &mut framed,
                                    true,
                                    piece,
                                    begin,
                                    length,
                                )
                                .await?;
                            } else {
                                anyhow::bail!("v2 peer exceeded the bounded upload queue");
                            }
                        } else {
                            pending_upload_requests.push_back(request);
                            start_v2_upload_reads(
                                &mut upload_reads,
                                &mut pending_upload_requests,
                                &context,
                                &state,
                            );
                        }
                    }
                    Message::Cancel { piece, begin, length } => {
                        let request = V2UploadRequest {
                            piece,
                            begin,
                            length,
                        };
                        if let Some(index) = pending_upload_requests
                            .iter()
                            .position(|pending| *pending == request)
                        {
                            let _ = pending_upload_requests.remove(index);
                            reject_v2_request(
                                &context,
                                &mut framed,
                                remote_supports_fast,
                                piece,
                                begin,
                                length,
                            )
                            .await?;
                        }
                    }
                    Message::Reject { piece, begin, length } => {
                        release_v2_rejected_request(
                            &mut outstanding,
                            piece,
                            begin,
                            length,
                        );
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    // BEP 6 Fast extension: TorrentNG does not bypass its
                    // normal choke/request policy for this optional hint.
                    Message::AllowedFast(_) => {}
                    Message::Piece { piece, begin, data } => {
                        handle_v2_piece(
                            &context,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            &mut transfer_jobs,
                            &mut pending_block_completions,
                            piece,
                            begin,
                            data,
                        ).await?;
                        start_ready_v2_piece_jobs(
                            &context,
                            &mut assemblies,
                            &mut pending_piece_completions,
                            &mut transfer_jobs,
                        );
                        update_v2_interest(
                            &context,
                            &mut framed,
                            &state,
                            &mut local_interested,
                            remote_availability_known,
                        ).await?;
                        fill_v2_requests(
                            &context,
                            &mut framed,
                            &state,
                            &mut outstanding,
                            &mut assemblies,
                            transfer_jobs.len(),
                            &pending_block_completions,
                            &pending_piece_completions,
                        ).await?;
                    }
                    Message::HashRequest { pieces_root, base_layer, index, length, proof_layers } => {
                        serve_v2_hash_request(
                            &context,
                            &mut framed,
                            pieces_root,
                            base_layer,
                            index,
                            length,
                            proof_layers,
                        ).await?;
                    }
                    Message::Hashes { .. } | Message::HashReject { .. } => {}
                    Message::Extended { ext_id: EXT_HANDSHAKE_ID, payload } => {
                        let _ = ExtensionHandshake::parse(&payload);
                    }
                    Message::Extended { .. } | Message::KeepAlive => {}
                }
            }
            _ = keepalive.tick() => {
                if last_activity.elapsed() >= V2_PEER_IDLE_TIMEOUT {
                    anyhow::bail!("v2 peer {addr} idle timeout");
                }
                if !send_v2_peer_message(&context, &mut framed, Message::KeepAlive).await? {
                    return Ok(());
                }
            }
            _ = memory_retry.tick() => {
                fill_v2_requests(
                    &context,
                    &mut framed,
                    &state,
                    &mut outstanding,
                    &mut assemblies,
                    transfer_jobs.len(),
                    &pending_block_completions,
                    &pending_piece_completions,
                )
                .await?;
            }
            transfer_result = transfer_jobs.next(), if !transfer_jobs.is_empty() => {
                let Some(completion) = transfer_result else {
                    continue;
                };
                match completion {
                    V2TransferCompletion::BlockStored(completion) => {
                        if let Err(error) = apply_v2_block_completion(
                            completion,
                            &mut assemblies,
                            &mut pending_block_completions,
                        ) {
                            return Err(error);
                        }
                    }
                    V2TransferCompletion::BlockAdmissionCancelled { piece, begin } => {
                        apply_v2_block_admission_cancelled(
                            piece,
                            begin,
                            &mut assemblies,
                            &mut pending_block_completions,
                        );
                        if context.peer_control.generation() != shutdown_generation {
                            return Ok(());
                        }
                    }
                    V2TransferCompletion::PieceVerified(completion) => {
                        apply_v2_piece_completion(
                            &context,
                            &state,
                            &mut pending_piece_completions,
                            completion,
                        ).await;
                    }
                }
                start_ready_v2_piece_jobs(
                    &context,
                    &mut assemblies,
                    &mut pending_piece_completions,
                    &mut transfer_jobs,
                );
                if let Ok(mut peer) = state.lock() {
                    update_v2_assembly_stats(
                        &mut peer,
                        &assemblies,
                        &pending_block_completions,
                        &pending_piece_completions,
                    );
                }
                update_v2_interest(
                    &context,
                    &mut framed,
                    &state,
                    &mut local_interested,
                    remote_availability_known,
                ).await?;
                fill_v2_requests(
                    &context,
                    &mut framed,
                    &state,
                    &mut outstanding,
                    &mut assemblies,
                    transfer_jobs.len(),
                    &pending_block_completions,
                    &pending_piece_completions,
                ).await?;
            }
            upload_result = upload_reads.next(), if !upload_reads.is_empty() => {
                let Some(completion) = upload_result else {
                    continue;
                };
                let request = completion.request;
                match completion.outcome {
                    V2UploadOutcome::Ready(block) if v2_upload_can_serve(&context, &state) => {
                        let V2LeasedUploadBlock { data, _lease } = block;
                        let bytes = data.len() as u64;
                        if !send_v2_peer_message(
                            &context,
                            &mut framed,
                            Message::Piece {
                                piece: request.piece,
                                begin: request.begin,
                                data,
                            },
                        )
                        .await?
                        {
                            return Ok(());
                        }
                        drop(_lease);
                        if let Ok(mut peer) = state.lock() {
                            peer.record_transfer(true, bytes);
                        }
                        let _ = context
                            .events
                            .send(V2PeerEvent::Uploaded {
                                peer: context.peer,
                                bytes,
                            })
                            .await;
                    }
                    V2UploadOutcome::Ready(_) | V2UploadOutcome::Rejected => {
                        reject_v2_request(
                            &context,
                            &mut framed,
                            remote_supports_fast,
                            request.piece,
                            request.begin,
                            request.length,
                        )
                        .await?;
                    }
                    V2UploadOutcome::RateInterrupted => {
                        if v2_upload_can_serve(&context, &state)
                            && pending_upload_requests.len() + upload_reads.len()
                                < V2_MAX_UPLOAD_WORK
                        {
                            pending_upload_requests.push_front(request);
                        } else if v2_upload_can_serve(&context, &state) {
                            // The active job was just removed, so this can
                            // only occur if the total-work invariant drifted.
                            // End a legacy connection rather than silently
                            // losing its request; Fast peers get the defined
                            // overload response.
                            if remote_supports_fast {
                                reject_v2_request(
                                    &context,
                                    &mut framed,
                                    true,
                                    request.piece,
                                    request.begin,
                                    request.length,
                                )
                                .await?;
                            } else {
                                anyhow::bail!("v2 upload work accounting exceeded its bound");
                            }
                        } else {
                            reject_v2_request(
                                &context,
                                &mut framed,
                                remote_supports_fast,
                                request.piece,
                                request.begin,
                                request.length,
                            )
                            .await?;
                        }
                    }
                    V2UploadOutcome::Failed(error) => {
                        warn!(
                            component = "torrent_v2",
                            operation = "read_upload_block",
                            peer = %addr,
                            piece = request.piece,
                            begin = request.begin,
                            length = request.length,
                            result = "error",
                            error = %error,
                            "failed to read pure-v2 upload block"
                        );
                        reject_v2_request(
                            &context,
                            &mut framed,
                            remote_supports_fast,
                            request.piece,
                            request.begin,
                            request.length,
                        )
                        .await?;
                    }
                }
                start_v2_upload_reads(
                    &mut upload_reads,
                    &mut pending_upload_requests,
                    &context,
                    &state,
                );
            }
        }
        }
    }
    .await;
    context.peer_control.cancel();
    let drain_result = drain_v2_transfer_jobs(
        &context,
        &state,
        &mut assemblies,
        &mut pending_block_completions,
        &mut pending_piece_completions,
        &mut transfer_jobs,
    )
    .await;
    match protocol_result {
        Err(error) => {
            if let Err(drain_error) = drain_result {
                debug!(
                    component = "torrent_v2",
                    operation = "drain_peer_transfers",
                    peer = %context.peer,
                    result = "error",
                    error = %drain_error,
                    "peer transfer drain also failed while closing the protocol loop"
                );
            }
            Err(error)
        }
        Ok(()) => drain_result,
    }
}

async fn send_v2_have_bitmap(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    have: &V2Bitmap,
    remote_supports_fast: bool,
) -> anyhow::Result<bool> {
    let count = have.count_ones();
    if remote_supports_fast && have.len() > 0 && count == have.len() {
        if !send_v2_peer_message(context, framed, Message::HaveAll).await? {
            return Ok(false);
        }
    } else if remote_supports_fast && count == 0 {
        if !send_v2_peer_message(context, framed, Message::HaveNone).await? {
            return Ok(false);
        }
    } else if count > 0 {
        let bitfield_bytes = u64::try_from(have.len().div_ceil(8))
            .map_err(|_| anyhow::anyhow!("v2 peer bitfield length does not fit in u64"))?;
        // The bitfield is materialized as a Vec, then copied into the framed
        // writer. Reserve the wire peak before creating that peer-controlled
        // allocation; the steady-state bitmap lease does not cover it.
        let _wire_memory_lease = context
            .resources
            .try_acquire(
                MemoryClass::PeerBuffer,
                bitfield_bytes.saturating_mul(3).saturating_add(5),
            )
            .ok_or_else(|| anyhow::anyhow!("v2 peer bitfield allocation denied"))?;
        if !send_v2_peer_message(context, framed, Message::Bitfield(have.to_bitfield())).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn send_v2_peer_message(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    message: Message,
) -> anyhow::Result<bool> {
    // A peer removal must be able to drop a pending framed write before
    // joining this task; the protocol loop then closes the socket and drains
    // its already-admitted storage work.
    tokio::select! {
        biased;
        _ = context.peer_control.wait_for_change(context.peer_generation) => Ok(false),
        result = framed.send(message) => {
            result?;
            Ok(true)
        }
    }
}

/// Keep the local interest bit synchronized with the pieces this peer can
/// currently provide. Before the first availability message, retain the
/// normal optimistic `Interested` state; once a bitfield/HaveNone is known,
/// withdraw interest when no wanted piece can be requested.
async fn update_v2_interest(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    state: &Arc<Mutex<V2PeerState>>,
    local_interested: &mut bool,
    remote_availability_known: bool,
) -> anyhow::Result<()> {
    if context.peer_control.generation() != context.peer_generation {
        return Ok(());
    }
    let local_have = context.local_have.read().await.clone();
    let has_missing_wanted = (0..context.piece_map.piece_count)
        .any(|piece| context.piece_is_wanted(piece) && !local_have.get(piece as usize));
    let remote_have = state
        .lock()
        .map(|peer| Arc::clone(&peer.remote_have))
        .map_err(|_| anyhow::anyhow!("v2 peer state lock poisoned"))?;
    let should_be_interested = has_missing_wanted
        && (!remote_availability_known
            || (0..context.piece_map.piece_count).any(|piece| {
                remote_have.get(piece as usize)
                    && context.piece_is_wanted(piece)
                    && !local_have.get(piece as usize)
            }));
    if should_be_interested == *local_interested {
        return Ok(());
    }
    if !send_v2_peer_message(
        context,
        framed,
        if should_be_interested {
            Message::Interested
        } else {
            Message::NotInterested
        },
    )
    .await?
    {
        return Ok(());
    }
    *local_interested = should_be_interested;
    Ok(())
}

async fn fill_v2_requests(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    state: &Arc<Mutex<V2PeerState>>,
    outstanding: &mut HashMap<(u32, u32), V2OutstandingRequest>,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    active_transfer_jobs: usize,
    pending_block_completions: &HashMap<(u32, u32), u64>,
    pending_piece_completions: &HashMap<u32, u64>,
) -> anyhow::Result<()> {
    if context.peer_control.generation() != context.peer_generation {
        return Ok(());
    }
    let choked = state.lock().map(|peer| peer.choked).unwrap_or(true);
    if choked {
        return Ok(());
    }
    let remote_have = state
        .lock()
        .map(|peer| Arc::clone(&peer.remote_have))
        .map_err(|_| anyhow::anyhow!("v2 peer state lock poisoned"))?;
    let local_have = context.local_have.read().await.clone();
    while outstanding.len() < V2_REQUEST_WINDOW {
        let mut selected = None;
        let ready_completions = pending_piece_completions.len()
            + assemblies
                .values()
                .filter(|assembly| assembly.is_ready())
                .count();
        let tracked_pieces = assemblies.len() + pending_piece_completions.len();
        if outstanding
            .len()
            .saturating_add(active_transfer_jobs)
            .saturating_add(ready_completions.saturating_sub(pending_piece_completions.len()))
            >= V2_REQUEST_WINDOW
        {
            break;
        }
        for piece in 0..context.piece_map.piece_count {
            let piece_index = piece as usize;
            if local_have.get(piece_index)
                || !remote_have.get(piece_index)
                || pending_piece_completions.contains_key(&piece)
            {
                continue;
            }
            let Ok(region) = context.piece_map.piece_to_file(piece) else {
                continue;
            };
            if region.pad || !context.file_is_wanted(region.file_index) {
                continue;
            }
            let is_new_assembly = !assemblies.contains_key(&piece);
            let assembly = if let Some(assembly) = assemblies.get(&piece) {
                if assembly.is_complete() {
                    continue;
                }
                if assembly.discard_when_idle {
                    continue;
                }
                None
            } else {
                if ready_completions >= V2_MAX_PENDING_PIECE_JOBS
                    || tracked_pieces >= V2_MAX_TRACKED_PIECES_PER_PEER
                {
                    continue;
                }
                Some(V2PieceAssembly::new(
                    piece,
                    region.length,
                    context.assembly_cap_bytes,
                    &context.resources,
                )?)
            };
            if let Some(assembly) = assembly {
                assemblies.insert(piece, assembly);
            }
            let Some(begin) = assemblies
                .get(&piece)
                .and_then(|assembly| assembly.next_unrequested(piece, outstanding))
            else {
                // Every missing block is already on the wire; try the next piece.
                continue;
            };
            let length = context
                .piece_map
                .validate_request(piece, begin, rt_peer_wire::message::MAX_BLOCK_SIZE)
                .map(|region| region.length)
                .or_else(|_| {
                    context
                        .piece_map
                        .piece_len(piece)
                        .ok()
                        .and_then(|length| length.checked_sub(begin))
                        .ok_or(rt_piece_map::PieceMapError::ZeroRequestLength)
                })?;
            selected = Some((piece, begin, length, is_new_assembly));
            break;
        }
        let Some((piece, begin, length, is_new_assembly)) = selected else {
            break;
        };
        let Some(memory_lease) = context
            .resources
            .try_acquire(MemoryClass::PeerBuffer, u64::from(length))
        else {
            if is_new_assembly {
                assemblies.remove(&piece);
            }
            break;
        };
        if !send_v2_peer_message(
            context,
            framed,
            Message::Request {
                piece,
                begin,
                length,
            },
        )
        .await?
        {
            return Ok(());
        }
        outstanding.insert(
            (piece, begin),
            V2OutstandingRequest {
                length,
                _memory_lease: Some(memory_lease),
            },
        );
    }
    if let Ok(mut peer) = state.lock() {
        peer.outstanding = outstanding.len();
        update_v2_assembly_stats(
            &mut peer,
            assemblies,
            pending_block_completions,
            pending_piece_completions,
        );
    }
    Ok(())
}

fn update_v2_assembly_stats(
    peer: &mut V2PeerState,
    assemblies: &HashMap<u32, V2PieceAssembly>,
    pending_block_completions: &HashMap<(u32, u32), u64>,
    pending_piece_completions: &HashMap<u32, u64>,
) {
    peer.assembly_buffers = (assemblies.len() + pending_piece_completions.len()) as u64;
    peer.assembly_bytes = assemblies
        .values()
        .filter_map(|assembly| assembly.data.as_ref().map(|data| data.len() as u64))
        .sum::<u64>()
        .saturating_add(pending_block_completions.values().copied().sum())
        .saturating_add(pending_piece_completions.values().copied().sum());
}

fn apply_v2_block_completion(
    completion: V2BlockCompletion,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    pending_block_completions: &mut HashMap<(u32, u32), u64>,
) -> anyhow::Result<()> {
    pending_block_completions.remove(&(completion.piece, completion.begin));
    let error = completion.result.err();
    if let Some(assembly) = assemblies.get_mut(&completion.piece) {
        assembly.pending_blocks = assembly.pending_blocks.saturating_sub(1);
        if error.is_some() {
            assembly.failed = true;
            assembly.discard_when_idle = true;
        }
    }
    let remove_assembly = assemblies.get(&completion.piece).is_some_and(|assembly| {
        assembly.pending_blocks == 0
            && (assembly.failed || (assembly.discard_when_idle && !assembly.is_complete()))
    });
    if remove_assembly {
        assemblies.remove(&completion.piece);
    }
    if let Some(error) = error {
        anyhow::bail!(
            "v2 block write failed for piece {} offset {}: {}",
            completion.piece,
            completion.begin,
            error
        );
    }
    Ok(())
}

fn apply_v2_block_admission_cancelled(
    piece: u32,
    begin: u32,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    pending_block_completions: &mut HashMap<(u32, u32), u64>,
) {
    pending_block_completions.remove(&(piece, begin));
    if let Some(assembly) = assemblies.get_mut(&piece) {
        assembly.pending_blocks = assembly.pending_blocks.saturating_sub(1);
        assembly.failed = true;
        assembly.discard_when_idle = true;
    }
    if assemblies
        .get(&piece)
        .is_some_and(|assembly| assembly.pending_blocks == 0 && assembly.failed)
    {
        assemblies.remove(&piece);
    }
}

fn start_ready_v2_piece_jobs(
    context: &V2PeerContext,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    pending_piece_completions: &mut HashMap<u32, u64>,
    transfer_jobs: &mut futures::stream::FuturesUnordered<V2TransferTask>,
) {
    while transfer_jobs.len() < V2_MAX_PENDING_TRANSFER_JOBS
        && pending_piece_completions.len() < V2_MAX_PENDING_PIECE_JOBS
    {
        let piece = assemblies
            .iter()
            .filter_map(|(piece, assembly)| assembly.is_ready().then_some(*piece))
            .min();
        let Some(piece) = piece else {
            break;
        };
        let Some(assembly) = assemblies.remove(&piece) else {
            continue;
        };
        let bytes = u64::from(assembly.region_len);
        let job_context = context.clone();
        pending_piece_completions.insert(piece, bytes);
        transfer_jobs.push(Box::pin(async move {
            let result = complete_v2_piece_job(&job_context, piece, assembly).await;
            V2TransferCompletion::PieceVerified(V2PieceCompletion {
                piece,
                bytes,
                result,
            })
        }));
    }
}

async fn apply_v2_piece_completion(
    context: &V2PeerContext,
    state: &Arc<Mutex<V2PeerState>>,
    pending_piece_completions: &mut HashMap<u32, u64>,
    completion: V2PieceCompletion,
) {
    pending_piece_completions.remove(&completion.piece);
    match completion.result {
        Ok(()) => {
            let newly_available = {
                let mut local_have = context.local_have.write().await;
                let local_have = Arc::make_mut(&mut *local_have);
                if local_have.get(completion.piece as usize) {
                    false
                } else {
                    local_have.set(completion.piece as usize, true);
                    true
                }
            };
            if newly_available {
                let _ = context.have_updates.send(());
                let _ = context
                    .events
                    .send(V2PeerEvent::Downloaded {
                        peer: context.peer,
                        bytes: completion.bytes,
                    })
                    .await;
                if let Ok(mut peer) = state.lock() {
                    peer.record_transfer(false, completion.bytes);
                }
            }
        }
        Err(error) => {
            warn!(
                component = "torrent_v2",
                operation = "verify_and_store_piece",
                peer = %context.peer,
                piece = completion.piece,
                result = "error",
                error = %error,
                "failed to verify or store pure-v2 piece"
            );
        }
    }
}

async fn drain_v2_transfer_jobs(
    context: &V2PeerContext,
    state: &Arc<Mutex<V2PeerState>>,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    pending_block_completions: &mut HashMap<(u32, u32), u64>,
    pending_piece_completions: &mut HashMap<u32, u64>,
    transfer_jobs: &mut futures::stream::FuturesUnordered<V2TransferTask>,
) -> anyhow::Result<()> {
    let mut first_error = None;
    while let Some(completion) = transfer_jobs.next().await {
        match completion {
            V2TransferCompletion::BlockStored(completion) => {
                if let Err(error) =
                    apply_v2_block_completion(completion, assemblies, pending_block_completions)
                {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            V2TransferCompletion::BlockAdmissionCancelled { piece, begin } => {
                apply_v2_block_admission_cancelled(
                    piece,
                    begin,
                    assemblies,
                    pending_block_completions,
                );
            }
            V2TransferCompletion::PieceVerified(completion) => {
                apply_v2_piece_completion(context, state, pending_piece_completions, completion)
                    .await;
            }
        }
        start_ready_v2_piece_jobs(
            context,
            assemblies,
            pending_piece_completions,
            transfer_jobs,
        );
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn complete_v2_piece_job(
    context: &V2PeerContext,
    piece: u32,
    mut assembly: V2PieceAssembly,
) -> anyhow::Result<()> {
    let extent = PieceGeometry::piece_extent(&context.piece_map, piece)
        .ok_or_else(|| anyhow::anyhow!("v2 piece {piece} is outside the piece map"))?;
    if extent.length != assembly.region_len {
        anyhow::bail!("v2 piece assembly length does not match its mapped extent");
    }
    let region = context
        .piece_map
        .piece_to_file(piece)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let file = context
        .meta
        .files
        .iter()
        .find(|file| file.index == region.file_index)
        .ok_or_else(|| anyhow::anyhow!("v2 file {} not found", region.file_index))?;
    let expected_hash = expected_v2_piece_hash(&context.meta, file, piece)?
        .ok_or_else(|| anyhow::anyhow!("v2 piece {piece} has no expected merkle hash"))?;
    let in_memory = assembly.data.is_some();
    let memory_lease = assembly
        ._memory_lease
        .take()
        .ok_or_else(|| anyhow::anyhow!("v2 piece assembly lost its memory lease"))?;
    let (payload_lease, _assembly_lease) = if in_memory {
        (Some(Arc::new(memory_lease)), None)
    } else {
        (None, Some(memory_lease))
    };
    let content = if let Some(data) = assembly.data.take() {
        Bytes::from(data)
    } else {
        let path = region.path.resolve(&context.save_root);
        scheduled_read_owned(
            &context.storage,
            IoClass::PeerRead,
            &path,
            region.file_offset,
            extent.length as usize,
        )
        .await?
        .into_bytes()
    };
    if content.len() != extent.length as usize {
        anyhow::bail!("v2 piece read length does not match map");
    }
    let mut leaves = context
        .storage
        .hash_v2_leaves(content.clone(), V2_BLOCK_SIZE)
        .await?;
    if file.length > context.meta.piece_length {
        let piece_leaf_count = usize::try_from(context.meta.piece_length / V2_BLOCK_SIZE as u64)
            .map_err(|error| anyhow::anyhow!("v2 piece leaf count does not fit usize: {error}"))?;
        if leaves.len() > piece_leaf_count {
            anyhow::bail!("v2 piece has more leaves than its piece length permits");
        }
        leaves.resize(piece_leaf_count, [0u8; 32]);
    }
    let actual_hash = context.storage.hash_v2_root(leaves).await?;
    if actual_hash != expected_hash {
        anyhow::bail!("v2 piece {piece} merkle hash mismatch");
    }
    if let Some(payload_lease) = payload_lease {
        let path = region.path.resolve(&context.save_root);
        for start in (0..content.len()).step_by(rt_peer_wire::message::MAX_BLOCK_SIZE as usize) {
            let end = content
                .len()
                .min(start.saturating_add(rt_peer_wire::message::MAX_BLOCK_SIZE as usize));
            let begin = u64::try_from(start)
                .map_err(|error| anyhow::anyhow!("v2 piece write offset overflow: {error}"))?;
            let offset = region
                .file_offset
                .checked_add(begin)
                .ok_or_else(|| anyhow::anyhow!("v2 piece write offset overflow"))?;
            scheduled_write_with_shared_payload_lease(
                &context.storage,
                IoClass::PeerWrite,
                &path,
                offset,
                content.slice(start..end),
                true,
                Arc::clone(&payload_lease),
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_v2_piece(
    context: &V2PeerContext,
    state: &Arc<Mutex<V2PeerState>>,
    outstanding: &mut HashMap<(u32, u32), V2OutstandingRequest>,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
    transfer_jobs: &mut futures::stream::FuturesUnordered<V2TransferTask>,
    pending_block_completions: &mut HashMap<(u32, u32), u64>,
    piece: u32,
    begin: u32,
    data: Bytes,
) -> anyhow::Result<()> {
    if context.peer_control.generation() != context.peer_generation {
        return Ok(());
    }
    let Some(request) = outstanding.remove(&(piece, begin)) else {
        anyhow::bail!("unsolicited v2 piece block");
    };
    let expected = request.length;
    let request_memory_lease = request._memory_lease;
    if expected != data.len() as u32 {
        anyhow::bail!("v2 piece response length does not match request");
    }
    // Keep the wire request and the complete verification unit typed
    // separately. `block_region` maps only the received range to storage;
    // assembly, read-back, hashing, and progress use the full piece extent.
    let block_extent = BlockExtent::new(piece, begin, expected);
    context
        .piece_map
        .validate_request_block(block_extent)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let piece_extent = PieceGeometry::piece_extent(&context.piece_map, piece)
        .ok_or_else(|| anyhow::anyhow!("v2 piece {piece} is outside the piece map"))?;
    let block_region = context
        .piece_map
        .validate_request(piece, begin, expected)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let region = context
        .piece_map
        .piece_to_file(piece)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    if region.length != piece_extent.length {
        anyhow::bail!("v2 piece map length disagrees with typed piece extent");
    }
    if let std::collections::hash_map::Entry::Vacant(entry) = assemblies.entry(piece) {
        entry.insert(V2PieceAssembly::new(
            piece_extent.piece,
            piece_extent.length,
            context.assembly_cap_bytes,
            &context.resources,
        )?);
    }
    let (accepted, in_memory) = {
        let assembly = assemblies
            .get_mut(&piece)
            .ok_or_else(|| anyhow::anyhow!("v2 piece assembly disappeared"))?;
        let accepted = assembly.accept_block(begin, &data)?;
        if accepted {
            assembly.pending_blocks = assembly.pending_blocks.saturating_add(1);
        }
        (accepted, assembly.data.is_some())
    };
    if !accepted {
        return Ok(());
    }
    pending_block_completions.insert(
        (piece, begin),
        if in_memory { 0 } else { u64::from(expected) },
    );
    let block_write = if in_memory {
        drop(request_memory_lease);
        None
    } else {
        Some((
            data,
            block_region.path.resolve(&context.save_root),
            block_region.file_offset,
            request_memory_lease,
        ))
    };
    let peer_generation = context.peer_generation;
    let job_context = context.clone();
    transfer_jobs.push(Box::pin(async move {
        if !job_context
            .network_budget
            .download()
            .acquire_or_cancelled(
                u64::from(expected),
                &job_context.peer_control,
                peer_generation,
            )
            .await
        {
            return V2TransferCompletion::BlockAdmissionCancelled { piece, begin };
        }
        let result = if let Some((data, path, offset, _memory_lease)) = block_write {
            let result = scheduled_write(
                &job_context.storage,
                IoClass::PeerWrite,
                &path,
                offset,
                data,
                true,
            )
            .await
            .map_err(|error| error.to_string());
            drop(_memory_lease);
            result
        } else {
            Ok(())
        };
        V2TransferCompletion::BlockStored(V2BlockCompletion {
            piece,
            begin,
            result,
        })
    }));
    if let Ok(mut peer) = state.lock() {
        peer.outstanding = outstanding.len();
    }
    Ok(())
}

fn v2_upload_can_serve(context: &V2PeerContext, state: &Arc<Mutex<V2PeerState>>) -> bool {
    state
        .lock()
        .map(|peer| !peer.upload_choked)
        .unwrap_or(false)
        && v2_upload_slot_selected(context)
}

fn start_v2_upload_reads(
    upload_reads: &mut futures::stream::FuturesUnordered<V2UploadReadTask>,
    pending: &mut VecDeque<V2UploadRequest>,
    context: &V2PeerContext,
    state: &Arc<Mutex<V2PeerState>>,
) {
    if context.peer_control.generation() != context.peer_generation {
        return;
    }
    while upload_reads.len() < V2_MAX_PENDING_UPLOAD_READS
        && !pending.is_empty()
        && v2_upload_can_serve(context, state)
    {
        let Some(request) = pending.pop_front() else {
            break;
        };
        let context = context.clone();
        let state = Arc::clone(state);
        upload_reads.push(
            async move {
                let outcome = read_v2_upload_block(&context, &state, request).await;
                V2UploadCompletion { request, outcome }
            }
            .boxed(),
        );
    }
}

async fn read_v2_upload_block(
    context: &V2PeerContext,
    state: &Arc<Mutex<V2PeerState>>,
    request: V2UploadRequest,
) -> V2UploadOutcome {
    let block_extent = BlockExtent::new(request.piece, request.begin, request.length);
    if PieceGeometry::validate_served_block(&context.piece_map, block_extent).is_err() {
        return V2UploadOutcome::Rejected;
    }
    let region =
        match context
            .piece_map
            .validate_request(request.piece, request.begin, request.length)
        {
            Ok(region) => region,
            Err(_) => return V2UploadOutcome::Rejected,
        };
    if !v2_upload_can_serve(context, state) {
        return V2UploadOutcome::Rejected;
    }
    if !region.pad && !context.local_have.read().await.get(request.piece as usize) {
        return V2UploadOutcome::Rejected;
    }
    let Some(lease) = reserve_v2_upload_bytes(&context.resources, region.length) else {
        return V2UploadOutcome::Rejected;
    };

    let data = if region.pad {
        // BEP 47 padding is synthetic but still consumes a bounded upload
        // buffer until the serialized peer writer finishes with this block.
        Bytes::from(vec![0u8; region.length as usize])
    } else {
        let path = region.path.resolve(&context.save_root);
        match scheduled_read_owned(
            &context.storage,
            IoClass::PeerRead,
            &path,
            region.file_offset,
            region.length as usize,
        )
        .await
        {
            Ok(data) => data.into_bytes(),
            Err(error) => return V2UploadOutcome::Failed(error.into()),
        }
    };

    // Capture before the final slot check: a choke that races between the
    // check and the wait changes this generation and wakes the waiter.
    let control_generation = context.upload_control.generation();
    if !v2_upload_can_serve(context, state) {
        return V2UploadOutcome::Rejected;
    }
    let upload_budget = context.network_budget.upload();
    let admitted = upload_budget
        .acquire_or_cancelled(
            u64::from(region.length),
            &context.upload_control,
            control_generation,
        )
        .await;
    if !admitted {
        return V2UploadOutcome::RateInterrupted;
    }
    if !v2_upload_can_serve(context, state) {
        return V2UploadOutcome::Rejected;
    }
    V2UploadOutcome::Ready(V2LeasedUploadBlock {
        data,
        _lease: lease,
    })
}

async fn reject_v2_request(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    remote_supports_fast: bool,
    piece: u32,
    begin: u32,
    length: u32,
) -> anyhow::Result<()> {
    if let Some(message) = v2_request_rejection_message(remote_supports_fast, piece, begin, length)
    {
        let _ = send_v2_peer_message(context, framed, message).await?;
    }
    Ok(())
}

fn v2_request_rejection_message(
    remote_supports_fast: bool,
    piece: u32,
    begin: u32,
    length: u32,
) -> Option<Message> {
    remote_supports_fast.then_some(Message::Reject {
        piece,
        begin,
        length,
    })
}

fn apply_v2_remote_choke(
    remote_supports_fast: bool,
    outstanding: &mut HashMap<(u32, u32), V2OutstandingRequest>,
    assemblies: &mut HashMap<u32, V2PieceAssembly>,
) {
    // BEP 6 no longer treats Choke as a response to outstanding requests.
    // Their Piece/Reject responses remain valid, so keep both the request
    // coordinates and piece assembly state for Fast peers. Legacy Choke ends
    // outstanding obligations, but any accepted block still being admitted or
    // written keeps its assembly until that bounded job returns.
    if !remote_supports_fast {
        outstanding.clear();
        assemblies.retain(|_, assembly| {
            if assembly.pending_blocks > 0 {
                assembly.discard_when_idle = true;
                true
            } else {
                false
            }
        });
    }
}

fn release_v2_rejected_request(
    outstanding: &mut HashMap<(u32, u32), V2OutstandingRequest>,
    piece: u32,
    begin: u32,
    length: u32,
) -> bool {
    let coordinate = (piece, begin);
    match outstanding.entry(coordinate) {
        std::collections::hash_map::Entry::Occupied(entry) if entry.get().length == length => {
            drop(entry.remove());
            true
        }
        _ => false,
    }
}

fn reserve_v2_upload_bytes(resources: &ResourceGovernor, bytes: u32) -> Option<MemoryLease> {
    resources.try_acquire(MemoryClass::PeerBuffer, u64::from(bytes))
}

async fn serve_v2_hash_request(
    context: &V2PeerContext,
    framed: &mut PeerIo,
    pieces_root: [u8; 32],
    base_layer: u32,
    index: u32,
    length: u32,
    proof_layers: u32,
) -> anyhow::Result<()> {
    let reject = || Message::HashReject {
        pieces_root,
        base_layer,
        index,
        length,
        proof_layers,
    };
    let Some(file) = context
        .meta
        .files
        .iter()
        .find(|file| file.pieces_root == Some(pieces_root))
    else {
        let _ = send_v2_peer_message(context, framed, reject()).await?;
        return Ok(());
    };
    let leaf_count = file.length.div_ceil(V2_BLOCK_SIZE as u64).max(1);
    let Some(_) = hash_request_bounds(
        usize::try_from(leaf_count).unwrap_or(usize::MAX),
        base_layer,
        index,
        length,
        proof_layers,
    ) else {
        let _ = send_v2_peer_message(context, framed, reject()).await?;
        return Ok(());
    };
    let file_is_available = if file.pad {
        true
    } else {
        tokio::select! {
            biased;
            _ = context.peer_control.wait_for_change(context.peer_generation) => {
                return Ok(());
            }
            available = hash_request_file_is_available(context, file, base_layer, index, length) => available,
        }
    };
    if !file_is_available {
        let _ = send_v2_peer_message(context, framed, reject()).await?;
        return Ok(());
    }
    let layers_result = tokio::select! {
        biased;
        _ = context.peer_control.wait_for_change(context.peer_generation) => {
            return Ok(());
        }
        result = v2_proof_layers(context, file, pieces_root, leaf_count) => result,
    };
    let layers = match layers_result {
        Ok(Some(layers)) => layers,
        Ok(None) => {
            let _ = send_v2_peer_message(context, framed, reject()).await?;
            return Ok(());
        }
        Err(error) => {
            debug!(
                component = "torrent_v2",
                operation = "build_hash_exchange_layers",
                peer = %context.peer,
                file = ?file.path,
                result = "rejected",
                error = %error,
                "unable to build v2 hash-exchange layers"
            );
            let _ = send_v2_peer_message(context, framed, reject()).await?;
            return Ok(());
        }
    };
    let leaves_len = layers.first().map_or(0, Vec::len);
    let Some(hashes) = hashes_for_v2_request(
        &layers,
        usize::try_from(leaf_count)
            .unwrap_or(usize::MAX)
            .min(leaves_len),
        base_layer,
        index,
        length,
        proof_layers,
    ) else {
        let _ = send_v2_peer_message(context, framed, reject()).await?;
        return Ok(());
    };
    let _ = send_v2_peer_message(
        context,
        framed,
        Message::Hashes {
            pieces_root,
            base_layer,
            index,
            length,
            proof_layers,
            hashes,
        },
    )
    .await?;
    Ok(())
}

/// A hash request is useful only when all content pieces covered by its base
/// range have been announced. Serving a hash block from an unverified local
/// file would let a peer authenticate data that this client must not claim to
/// possess. Padding-only ranges need no local file because their hashes are
/// the protocol-defined zero padding nodes.
async fn hash_request_file_is_available(
    context: &V2PeerContext,
    file: &TorrentFileV2,
    base_layer: u32,
    index: u32,
    length: u32,
) -> bool {
    let Some(hash_span) = (V2_BLOCK_SIZE as u64).checked_shl(base_layer) else {
        return false;
    };
    let Some(start) = u64::from(index).checked_mul(hash_span) else {
        return false;
    };
    let Some(end) = start.checked_add(u64::from(length).saturating_mul(hash_span)) else {
        return false;
    };
    let content_end = end.min(file.length);
    if start >= content_end {
        return true;
    }
    let Some(first_offset) = file.piece_offset.checked_add(start) else {
        return false;
    };
    let Some(last_offset) = file.piece_offset.checked_add(content_end.saturating_sub(1)) else {
        return false;
    };
    if context.piece_map.piece_length == 0 {
        return false;
    }
    let first_piece = first_offset / context.piece_map.piece_length;
    let last_piece = last_offset / context.piece_map.piece_length;
    let have = context.local_have.read().await;
    (first_piece..=last_piece).all(|piece| {
        usize::try_from(piece)
            .ok()
            .is_some_and(|piece| have.get(piece))
    })
}

/// Build the `hashes` payload for a BEP 52 request. The requested base-layer
/// hashes are followed by the uncle hashes needed to reach the file root.
/// The first `log2(length) - 1` proof layers are implicit because the
/// requested power-of-two range already contains those complete child layers;
/// they remain counted in `proof_layers` as required by the wire format.
fn hashes_for_v2_request(
    layers: &[Vec<[u8; 32]>],
    leaf_count: usize,
    base_layer: u32,
    index: u32,
    length: u32,
    proof_layers: u32,
) -> Option<Vec<[u8; 32]>> {
    let (base, start, count, proof_count) =
        hash_request_bounds(leaf_count, base_layer, index, length, proof_layers)?;
    let layer = layers.get(base)?;
    let end = start.checked_add(count)?;
    if end > layer.len() {
        return None;
    }
    let padded_leaf_count = layers.first()?.len();
    let expected_layer_len = padded_leaf_count.checked_shr(base_layer)?;
    if expected_layer_len == 0 || layer.len() != expected_layer_len {
        return None;
    }

    let mut hashes = layer[start..end].to_vec();
    let omitted_layers = length.trailing_zeros() as usize;
    if omitted_layers <= proof_count {
        for proof_index in omitted_layers..=proof_count {
            let proof_layer = base.checked_add(proof_index)?;
            // `proof_layers` is the highest ancestor depth requested. The
            // root itself has no uncle, so hash_request_bounds excludes that
            // layer before this loop.
            let node_index = start.checked_shr(proof_index as u32)?;
            let sibling = node_index ^ 1;
            hashes.push(*layers.get(proof_layer)?.get(sibling)?);
        }
    }
    Some(hashes)
}

/// Validate a BEP 52 hash request against a file's padded Merkle-tree shape.
/// The returned proof-layer value is the inclusive ancestor depth used by the
/// wire format: for a two-hash request, `proof_layers = 2` asks for the two
/// uncles above the requested subtree when the root is at depth three.
fn hash_request_bounds(
    leaf_count: usize,
    base_layer: u32,
    index: u32,
    length: u32,
    proof_layers: u32,
) -> Option<(usize, usize, usize, usize)> {
    if leaf_count == 0 || length < 2 || !length.is_power_of_two() || !index.is_multiple_of(length) {
        return None;
    }
    if length > V2_MAX_HASH_REQUEST_LENGTH {
        return None;
    }
    let padded_leaf_count = leaf_count.checked_next_power_of_two()?;
    let tree_height = padded_leaf_count.trailing_zeros() as usize;
    let base = usize::try_from(base_layer).ok()?;
    if base > tree_height {
        return None;
    }
    let layer_len = padded_leaf_count.checked_shr(base_layer)?;
    let start = usize::try_from(index).ok()?;
    let count = usize::try_from(length).ok()?;
    let end = start.checked_add(count)?;
    if layer_len < count || end > layer_len {
        return None;
    }
    let proof_count = usize::try_from(proof_layers).ok()?;
    let max_proof_layer = tree_height.saturating_sub(base.saturating_add(1));
    if proof_count > max_proof_layer {
        return None;
    }
    Some((base, start, count, proof_count))
}

/// Merkle layers for `file`, from the proof cache or built by hashing the
/// file in bounded reads. Returns `None` when the content on disk does not
/// hash to the metainfo `pieces root`: unverified bytes must never be served
/// as authenticated hashes, and nothing is cached for them.
async fn v2_proof_layers(
    context: &V2PeerContext,
    file: &TorrentFileV2,
    pieces_root: [u8; 32],
    leaf_count: u64,
) -> anyhow::Result<Option<V2ProofLayers>> {
    if let Some(layers) = context.proof_cache.get(&pieces_root) {
        return Ok(Some(layers));
    }
    // All layers together are about twice the padded leaf layer.
    let padded_leaves = leaf_count
        .checked_next_power_of_two()
        .ok_or_else(|| anyhow::anyhow!("v2 leaf count overflow"))?;
    let layer_bytes = padded_leaves
        .saturating_mul(2)
        .saturating_mul(std::mem::size_of::<[u8; 32]>() as u64);
    let leaf_bytes = leaf_count.saturating_mul(std::mem::size_of::<[u8; 32]>() as u64);
    let Some(_hash_memory_lease) = context.resources.try_acquire(
        MemoryClass::PeerBuffer,
        layer_bytes
            .saturating_add(leaf_bytes)
            .saturating_add(V2_HASH_EXCHANGE_READ_BYTES as u64),
    ) else {
        return Ok(None);
    };
    let leaves = if file.pad {
        // BEP 47 padding is synthetic zero content. Its hash tree is still
        // part of the v2 file metadata when a creator included the padding
        // leaf, so serve it without requiring a local `.pad/*` file.
        v2_zero_leaf_hashes(file.length)?
    } else {
        read_v2_leaf_hashes(context, file).await?
    };
    let layers = merkle_layers(&leaves);
    drop(leaves);
    if layers.last().and_then(|layer| layer.first()) != Some(&pieces_root) {
        return Ok(None);
    }
    let layers = Arc::new(layers);
    context
        .proof_cache
        .insert(pieces_root, Arc::clone(&layers), layer_bytes);
    Ok(Some(layers))
}

/// Hash `file` into 16 KiB leaf hashes with bounded reads, so the payload is
/// never held in memory and the engine task yields between reads.
async fn read_v2_leaf_hashes(
    context: &V2PeerContext,
    file: &TorrentFileV2,
) -> anyhow::Result<Vec<[u8; 32]>> {
    let path = file.path.resolve(&context.save_root);
    let leaf_count = usize::try_from(file.length.div_ceil(V2_BLOCK_SIZE as u64).max(1))
        .map_err(|error| anyhow::anyhow!("v2 leaf count does not fit usize: {error}"))?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(leaf_count)
        .map_err(|error| anyhow::anyhow!("v2 hash exchange allocation failed: {error}"))?;
    if file.length == 0 {
        leaves.push(BlockHash::of(&[]).0);
        return Ok(leaves);
    }
    let mut offset = 0u64;
    while offset < file.length {
        let length = (file.length - offset).min(V2_HASH_EXCHANGE_READ_BYTES as u64) as usize;
        let read = scheduled_read_owned(&context.storage, IoClass::PeerRead, &path, offset, length)
            .await?;
        for chunk in read.as_slice().chunks(V2_BLOCK_SIZE) {
            leaves.push(BlockHash::of(chunk).0);
        }
        offset += length as u64;
    }
    Ok(leaves)
}

#[cfg(test)]
fn v2_leaf_hashes(data: &[u8]) -> Vec<[u8; 32]> {
    if data.is_empty() {
        return vec![BlockHash::of(&[]).0];
    }
    data.chunks(V2_BLOCK_SIZE)
        .map(|chunk| BlockHash::of(chunk).0)
        .collect()
}

fn v2_zero_leaf_hashes(length: u64) -> anyhow::Result<Vec<[u8; 32]>> {
    let leaf_count = usize::try_from(length.div_ceil(V2_BLOCK_SIZE as u64).max(1))
        .map_err(|error| anyhow::anyhow!("v2 padding leaf count does not fit usize: {error}"))?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(leaf_count)
        .map_err(|error| anyhow::anyhow!("v2 padding hash allocation failed: {error}"))?;
    if length == 0 {
        leaves.push(BlockHash::of(&[]).0);
        return Ok(leaves);
    }
    let full_zero_block = vec![0u8; V2_BLOCK_SIZE];
    let full_hash = BlockHash::of(&full_zero_block).0;
    leaves.resize(leaf_count, full_hash);
    let remainder = (length % V2_BLOCK_SIZE as u64) as usize;
    if remainder != 0 {
        leaves[leaf_count - 1] = BlockHash::of(&full_zero_block[..remainder]).0;
    }
    Ok(leaves)
}

#[cfg(test)]
fn actual_v2_piece_hash(data: &[u8], file_length: u64, piece_length: u64) -> [u8; 32] {
    if file_length <= piece_length {
        merkle_root(&v2_leaf_hashes(data))
    } else {
        piece_layer_hash(data, piece_length as usize).unwrap_or([0u8; 32])
    }
}

fn expected_v2_piece_hash(
    meta: &TorrentMetaV2,
    file: &TorrentFileV2,
    piece: u32,
) -> anyhow::Result<Option<[u8; 32]>> {
    if file.length <= meta.piece_length {
        return Ok(file.pieces_root);
    }
    let root = file
        .pieces_root
        .ok_or_else(|| anyhow::anyhow!("v2 layered file has no pieces root"))?;
    let first_piece = file.piece_offset / meta.piece_length;
    let index = piece
        .checked_sub(
            u32::try_from(first_piece).map_err(|_| anyhow::anyhow!("v2 piece offset overflow"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("v2 piece precedes its file"))? as usize;
    Ok(meta
        .piece_layers
        .get(&root)
        .and_then(|hashes| hashes.get(index))
        .copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_roundtrip_preserves_v2_shape() {
        let mut bitmap = V2Bitmap::new(10);
        bitmap.set(0, true);
        bitmap.set(9, true);
        let encoded = bitmap.to_bitfield();
        let mut decoded = V2Bitmap::new(10);
        decoded.copy_from_bitfield(&encoded).unwrap();
        assert_eq!(decoded.count_ones(), 2);
    }

    #[test]
    fn v2_upload_slot_changes_send_only_required_choke_transitions() {
        let state = Arc::new(Mutex::new(V2PeerState::new(0)));
        assert_eq!(set_v2_upload_unchoked(&state, false), None);
        assert_eq!(set_v2_upload_unchoked(&state, true), Some(Message::Unchoke));
        assert_eq!(set_v2_upload_unchoked(&state, true), None);
        assert_eq!(set_v2_upload_unchoked(&state, false), Some(Message::Choke));
        assert_eq!(set_v2_upload_unchoked(&state, false), None);
    }

    #[test]
    fn v2_upload_slot_does_not_carry_across_a_reconnected_peer_identity() {
        let prior_connection = PeerId::new();
        let replacement_connection = PeerId::new();
        let selected = HashSet::from([prior_connection]);

        assert!(v2_slot_selected(&selected, prior_connection));
        assert!(!v2_slot_selected(&selected, replacement_connection));
    }

    #[tokio::test]
    async fn v2_peer_reads_control_messages_while_transfer_jobs_wait_for_rate_budget() {
        use tokio::net::TcpListener;

        let temp = tempfile::tempdir().unwrap();
        let file_name = rt_path::SafeRelPath::from_name("payload.bin", false).unwrap();
        std::fs::write(
            file_name.resolve(temp.path()),
            vec![7u8; rt_peer_wire::message::MAX_BLOCK_SIZE as usize],
        )
        .unwrap();
        let piece_map = Arc::new(
            V2PieceMap::new(
                u64::from(rt_peer_wire::message::MAX_BLOCK_SIZE),
                vec![V2FileSpan {
                    file_index: 0,
                    path: file_name.clone(),
                    piece_offset: 0,
                    length: u64::from(rt_peer_wire::message::MAX_BLOCK_SIZE),
                    pad: false,
                }],
            )
            .unwrap(),
        );
        let meta = Arc::new(TorrentMetaV2 {
            info_hash_v2: [9; 32],
            announce: None,
            announce_list: Vec::new(),
            webseeds: Vec::new(),
            comment: None,
            created_by: None,
            creation_date: None,
            name: "upload-progress".to_owned(),
            piece_length: u64::from(rt_peer_wire::message::MAX_BLOCK_SIZE),
            files: vec![TorrentFileV2 {
                index: 0,
                length: u64::from(rt_peer_wire::message::MAX_BLOCK_SIZE),
                path: file_name,
                offset: 0,
                piece_offset: 0,
                pieces_root: None,
                pad: false,
            }],
            piece_layers: HashMap::new(),
            private: false,
            raw: Vec::new(),
        });
        let choker_id = PeerId::new();
        let (_slot_tx, slot_rx) = watch::channel(Arc::new(HashSet::from([choker_id])));
        let (have_updates, _) = watch::channel(());
        let resources = ResourceGovernor::new(Default::default());
        let storage = MountScheduler::new_for_path(
            StorageRootId::new(),
            temp.path(),
            &SchedulerConfig {
                profile: StorageProfile::Unknown,
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let mut local_have = V2Bitmap::new(1);
        local_have.set(0, true);
        let (events, mut event_rx) = mpsc::channel(8);
        let context = V2PeerContext {
            info_hash: [9; 20],
            peer: "127.0.0.1:6881".parse().unwrap(),
            choker_id,
            meta,
            piece_map,
            save_root: temp.path().to_path_buf(),
            storage: storage.clone(),
            resources,
            network_budget: GlobalNetworkBudget::new(8, Some(1), Some(1)),
            peer_control: RateLimitCancellation::default(),
            peer_generation: 0,
            upload_control: RateLimitCancellation::default(),
            local_have: Arc::new(RwLock::new(Arc::new(local_have))),
            have_updates,
            upload_unchoked: slot_rx,
            file_policy: Arc::new(HashMap::new()),
            assembly_cap_bytes: 1024 * 1024,
            events,
            proof_cache: Arc::new(V2ProofCache::default()),
        };
        let context_for_test = context.clone();
        let upload_budget = context.network_budget.upload();
        upload_budget.acquire(64 * 1024).await;
        let download_budget = context.network_budget.download();
        download_budget.acquire(64 * 1024).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = listener.local_addr().unwrap();
        let remote = TcpStream::connect(peer_addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let state = Arc::new(Mutex::new(V2PeerState::new(1)));
        state.lock().unwrap().upload_choked = false;
        let state_for_test = Arc::clone(&state);
        let peer = tokio::spawn(run_v2_peer_protocol(
            context,
            peer_addr,
            PeerIo::Tcp(Framed::new(server, PeerCodec::default())),
            state,
            false,
            false,
        ));
        let mut remote = Framed::new(remote, PeerCodec::default());
        assert!(matches!(
            remote.next().await.unwrap().unwrap(),
            Message::Bitfield(_)
        ));
        remote
            .send(Message::Request {
                piece: 0,
                begin: 0,
                length: rt_peer_wire::message::MAX_BLOCK_SIZE,
            })
            .await
            .unwrap();

        timeout(Duration::from_secs(2), async {
            while storage.stats().backend_read_ops_by_class[IoClass::PeerRead as usize] == 0 {
                tokio::task::yield_now().await;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .await
        .expect("v2 upload storage read did not complete");
        remote.send(Message::Interested).await.unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), event_rx.recv())
                .await
                .expect("v2 peer loop stalled behind upload rate admission"),
            Some(V2PeerEvent::InterestChanged)
        ));

        {
            let mut local_have = context_for_test.local_have.write().await;
            Arc::make_mut(&mut *local_have).set(0, false);
        }
        let _ = context_for_test.have_updates.send(());
        let mut remote_have = V2Bitmap::new(1);
        remote_have.set(0, true);
        remote
            .send(Message::Bitfield(remote_have.to_bitfield()))
            .await
            .unwrap();
        remote.send(Message::Unchoke).await.unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    remote.next().await.unwrap().unwrap(),
                    Message::Request {
                        piece: 0,
                        begin: 0,
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .expect("v2 download request was not issued");
        remote
            .send(Message::Piece {
                piece: 0,
                begin: 0,
                data: Bytes::from(vec![7u8; rt_peer_wire::message::MAX_BLOCK_SIZE as usize]),
            })
            .await
            .unwrap();
        remote.send(Message::NotInterested).await.unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), event_rx.recv())
                .await
                .expect("v2 peer loop stalled behind download rate admission"),
            Some(V2PeerEvent::InterestChanged)
        ));
        assert!(!context_for_test.local_have.read().await.get(0));
        assert_eq!(state_for_test.lock().unwrap().downloaded, 0);

        context_for_test.peer_control.cancel();
        assert!(timeout(Duration::from_secs(1), peer)
            .await
            .expect("v2 shutdown did not cancel a pending download-rate wait")
            .unwrap()
            .is_ok());
        assert!(!context_for_test.local_have.read().await.get(0));
        assert_eq!(state_for_test.lock().unwrap().downloaded, 0);
        assert!(event_rx.try_recv().is_err());
        drop(remote);
    }

    #[tokio::test]
    async fn v2_peer_shutdown_cancels_a_backpressured_socket_send() {
        use tokio::net::TcpListener;

        let content = vec![0x5Au8; rt_peer_wire::message::MAX_BLOCK_SIZE as usize];
        let raw = v2_raw_with_piece_length(&content, content.len());
        let (task, _fixture) =
            v2_task_not_started(quiet_v2_runtime(), &raw, content.len() as u64).await;
        let context = task.peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
        let send_context = context.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (remote, _) = listener.accept().await.unwrap();
        let mut framed = PeerIo::Tcp(Framed::new(client, PeerCodec::default()));
        let sender = tokio::spawn(async move {
            let data = Bytes::from(content);
            for completed in 0..8_192usize {
                let sent = send_v2_peer_message(
                    &send_context,
                    &mut framed,
                    Message::Piece {
                        piece: 0,
                        begin: 0,
                        data: data.clone(),
                    },
                )
                .await?;
                if !sent {
                    return Ok::<_, anyhow::Error>(Some(completed));
                }
            }
            Ok(None)
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !sender.is_finished(),
            "non-reading peer should leave the writer backpressured"
        );
        context.peer_control.cancel();
        let completed = timeout(Duration::from_secs(1), sender)
            .await
            .expect("shutdown should release a backpressured peer send")
            .unwrap()
            .unwrap();
        assert!(completed.is_some());
        drop(remote);
    }

    #[test]
    fn v2_piece_index_accounting_covers_overlapping_bitmap_replacement() {
        let one_bitmap = v2_bitmap_memory_bytes(65);
        assert_eq!(V2Bitmap::memory_bytes_for_len(65), one_bitmap as u64);
        assert_eq!(v2_piece_index_memory_bytes(65), one_bitmap * 2);
    }

    #[test]
    fn v2_reject_obligations_require_fast_negotiation() {
        assert!(v2_request_rejection_message(false, 3, 16_384, 8_192).is_none());
        assert!(matches!(
            v2_request_rejection_message(true, 3, 16_384, 8_192),
            Some(Message::Reject {
                piece: 3,
                begin: 16_384,
                length: 8_192,
            })
        ));
    }

    #[test]
    fn v2_choke_preserves_fast_request_obligations() {
        let resources = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig::default());
        let mut outstanding = HashMap::from([(
            (3, 0),
            V2OutstandingRequest {
                length: 4,
                _memory_lease: Some(resources.try_acquire(MemoryClass::PeerBuffer, 4).unwrap()),
            },
        )]);
        let mut assemblies = HashMap::new();
        assemblies.insert(3, V2PieceAssembly::new(3, 8, 1024, &resources).unwrap());
        let assembly_bytes =
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes;
        assert!(assembly_bytes > 0);

        apply_v2_remote_choke(true, &mut outstanding, &mut assemblies);
        assert_eq!(
            outstanding.get(&(3, 0)).map(|request| request.length),
            Some(4)
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            4,
            "Fast Choke preserves the on-wire response reservation"
        );
        assert!(assemblies.contains_key(&3));

        apply_v2_remote_choke(false, &mut outstanding, &mut assemblies);
        assert!(outstanding.is_empty());
        assert!(assemblies.is_empty());
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0,
            "legacy Choke releases terminal request reservations"
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes,
            0
        );
    }

    #[test]
    fn v2_reject_releases_only_matching_response_lease() {
        let resources = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig::default());
        let mut outstanding = HashMap::from([(
            (3, 16_384),
            V2OutstandingRequest {
                length: 8_192,
                _memory_lease: Some(
                    resources
                        .try_acquire(MemoryClass::PeerBuffer, 8_192)
                        .unwrap(),
                ),
            },
        )]);

        assert!(!release_v2_rejected_request(
            &mut outstanding,
            3,
            16_384,
            4_096
        ));
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            8_192,
            "a mismatched Reject must not release the outstanding response lease"
        );
        assert!(release_v2_rejected_request(
            &mut outstanding,
            3,
            16_384,
            8_192
        ));
        assert!(outstanding.is_empty());
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0,
            "a matching Reject terminates and releases the request lease"
        );
    }

    #[test]
    fn metainfo_tracker_tiers_are_not_flattened() {
        let meta = TorrentMetaV2 {
            info_hash_v2: [0; 32],
            announce: Some("https://primary.example/announce".to_owned()),
            announce_list: vec![
                vec![
                    "https://primary.example/announce".to_owned(),
                    "https://backup-a.example/announce".to_owned(),
                ],
                vec![
                    "https://backup-b.example/announce".to_owned(),
                    "https://backup-a.example/announce".to_owned(),
                ],
            ],
            webseeds: Vec::new(),
            comment: None,
            created_by: None,
            creation_date: None,
            name: "tiered".to_owned(),
            piece_length: 16 * 1024,
            files: Vec::new(),
            piece_layers: HashMap::new(),
            private: false,
            raw: Vec::new(),
        };

        let tiers = tracker_tiers_from_meta_v2(&meta);
        let urls = tiers
            .iter()
            .map(|tier| {
                tier.iter()
                    .map(|tracker| tracker.url.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                vec![
                    "https://primary.example/announce",
                    "https://backup-a.example/announce"
                ],
                vec!["https://backup-b.example/announce"],
            ]
        );
    }

    #[tokio::test]
    async fn private_tracker_failover_disconnects_v2_peers_and_clears_allowlist() {
        let temp = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        rt_db::migrate(&conn).unwrap();
        let db = DbExecutor::direct(Arc::new(Mutex::new(conn)));
        let resources = ResourceGovernor::new(Default::default());
        let meta = TorrentMetaV2 {
            info_hash_v2: [27; 32],
            announce: Some("https://tracker-a.example/announce".to_owned()),
            announce_list: vec![vec![
                "https://tracker-a.example/announce".to_owned(),
                "https://tracker-b.example/announce".to_owned(),
            ]],
            webseeds: Vec::new(),
            comment: None,
            created_by: None,
            creation_date: None,
            name: "private-v2-failover.bin".to_owned(),
            piece_length: 16 * 1024,
            files: Vec::new(),
            piece_layers: HashMap::new(),
            private: true,
            raw: Vec::new(),
        };
        let (_cmd_tx, cmd_rx) = mpsc::channel(1);
        let mut task = V2TorrentTask::new(
            meta,
            temp.path().to_path_buf(),
            false,
            TorrentState::Downloading,
            Arc::new(RwLock::new(SessionRegistry::new())),
            db,
            resources.clone(),
            cmd_rx,
            8,
            1024 * 1024,
            StorageIoConfig::default(),
            OutboundEgressPolicy::default(),
            6881,
            10,
            10,
            60,
            GlobalNetworkBudget::unlimited(),
            None,
            None,
        )
        .await;

        let peer: SocketAddr = "127.0.0.1:6881".parse().unwrap();
        let peer_task = tokio::spawn(std::future::pending::<()>());
        task.peers.insert(
            peer,
            V2PeerHandle {
                state: Arc::new(Mutex::new(V2PeerState::new(0))),
                choker_id: PeerId::new(),
                peer_control: RateLimitCancellation::default(),
                task: peer_task,
                _bitmap_memory_lease: resources.try_acquire(MemoryClass::PeerBuffer, 1).unwrap(),
            },
        );
        task.known_tracker_peers.insert(peer);

        task.advance_private_tracker_after_failure((0, 0)).await;

        assert_eq!(task.private_tracker_key, Some((0, 1)));
        assert!(task.peers.is_empty());
        assert!(task.known_tracker_peers.is_empty());
        assert!(task.tracker_tiers[0][1].is_due());
    }

    #[test]
    fn piece_assembly_accepts_the_full_peer_coordinate_range() {
        let resources = ResourceGovernor::new(Default::default());
        let block_count = 65_537usize;
        let region_len = u32::try_from(
            block_count
                .saturating_mul(rt_peer_wire::message::MAX_BLOCK_SIZE as usize)
                .saturating_sub(1),
        )
        .unwrap();
        let assembly = V2PieceAssembly::new(0, region_len, 0, &resources).unwrap();
        assert_eq!(assembly.received.len(), block_count);
    }

    #[test]
    fn v2_upload_payload_reservation_is_bounded() {
        let mut class_caps = [0; rt_metrics::MEMORY_CLASS_COUNT];
        class_caps[MemoryClass::PeerBuffer as usize] = 16;
        let resources = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 16,
            class_caps_bytes: class_caps,
            pressure_constrained_pct: 75,
            pressure_critical_pct: 90,
        });

        let lease = reserve_v2_upload_bytes(&resources, 16).expect("payload fits");
        assert_eq!(lease.bytes(), 16);
        assert!(reserve_v2_upload_bytes(&resources, 1).is_none());
        drop(lease);
        assert_eq!(resources.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn short_piece_uses_file_root_tree_not_piece_layer_padding() {
        let data = vec![7u8; V2_BLOCK_SIZE + 3];
        let actual = actual_v2_piece_hash(&data, data.len() as u64, 64 * 1024);
        assert_eq!(actual, merkle_root(&v2_leaf_hashes(&data)));
        assert_ne!(actual, piece_layer_hash(&data, 64 * 1024).unwrap());
    }

    #[test]
    fn hash_exchange_includes_only_required_uncles() {
        let leaves = (0..8)
            .map(|value| BlockHash::of(&[value]).0)
            .collect::<Vec<_>>();
        let layers = merkle_layers(&leaves);
        let hashes = hashes_for_v2_request(&layers, leaves.len(), 0, 0, 2, 2).unwrap();

        assert_eq!(hashes.len(), 4);
        assert_eq!(&hashes[..2], &layers[0][..2]);
        assert_eq!(hashes[2], layers[1][1]);
        assert_eq!(hashes[3], layers[2][1]);
        assert!(hashes_for_v2_request(&layers, leaves.len(), 0, 0, 2, 3).is_none());
    }

    #[test]
    fn hash_exchange_uses_zero_padding_at_file_end() {
        let leaves = (0..3)
            .map(|value| BlockHash::of(&[value]).0)
            .collect::<Vec<_>>();
        let layers = merkle_layers(&leaves);
        let hashes = hashes_for_v2_request(&layers, leaves.len(), 0, 2, 2, 1).unwrap();

        assert_eq!(hashes[0], layers[0][2]);
        assert_eq!(hashes[1], [0; 32]);
    }

    #[test]
    fn padding_hash_tree_uses_zero_content_without_a_file() {
        let leaves = v2_zero_leaf_hashes((V2_BLOCK_SIZE + 3) as u64).unwrap();
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0], BlockHash::of(&vec![0u8; V2_BLOCK_SIZE]).0);
        assert_eq!(leaves[1], BlockHash::of(&[0u8; 3]).0);
    }

    #[test]
    fn hash_exchange_allows_bounded_padding_at_file_end() {
        let leaves = (0..3)
            .map(|value| BlockHash::of(&[value]).0)
            .collect::<Vec<_>>();
        let layers = merkle_layers(&leaves);

        assert!(hashes_for_v2_request(&layers, leaves.len(), 0, 2, 2, 0).is_some());
        assert!(hashes_for_v2_request(&layers, leaves.len(), 0, 0, 2, 0).is_some());
    }

    #[test]
    fn hash_exchange_rejects_invalid_tree_coordinates_before_reading() {
        assert!(hash_request_bounds(8, 0, 0, 1, 0).is_none());
        assert!(hash_request_bounds(8, 0, 1, 2, 0).is_none());
        assert!(hash_request_bounds(8, 0, 0, 4, 2).is_some());
        assert!(hash_request_bounds(8, 0, 0, 4, 3).is_none());
        assert!(hash_request_bounds(8, 0, 4, 8, 0).is_none());
        assert!(hash_request_bounds(8, 4, 0, 2, 0).is_none());
        assert!(hash_request_bounds(1024, 0, 0, 1024, 0).is_none());
    }

    // ---- crash safety: completion gate -------------------------------------

    fn v2_raw_with_content(content: &[u8]) -> Vec<u8> {
        use rt_bencode::{encode, BValue};
        use rt_hash::{merkle_root, BlockHash};
        let piece_layers: Vec<[u8; 32]> = content
            .chunks(16_384)
            .map(|chunk| BlockHash::of(chunk).0)
            .collect();
        let pieces_root = merkle_root(&piece_layers);
        let mut layer_bytes = Vec::new();
        for hash in &piece_layers {
            layer_bytes.extend_from_slice(hash);
        }
        let leaf = BValue::Dict({
            let mut pairs: Vec<(&[u8], BValue<'_>)> = vec![
                (b"length", BValue::Int(content.len() as i64)),
                (b"pieces root", BValue::Bytes(&pieces_root)),
            ];
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            pairs
        });
        let file_tree = BValue::Dict(vec![(
            b"data.bin".as_ref(),
            BValue::Dict(vec![(b"".as_ref(), leaf)]),
        )]);
        let mut info: Vec<(&[u8], BValue<'_>)> = vec![
            (b"file tree", file_tree),
            (b"meta version", BValue::Int(2)),
            (b"name", BValue::Bytes(b"v2dir")),
            (b"piece length", BValue::Int(16_384)),
        ];
        info.sort_by(|a, b| a.0.cmp(b.0));
        let layers = BValue::Dict(vec![(&pieces_root[..], BValue::Bytes(&layer_bytes))]);
        let mut top: Vec<(&[u8], BValue<'_>)> = vec![
            (b"announce", BValue::Bytes(b"http://tracker.example/v2")),
            (b"info", BValue::Dict(info)),
            (b"piece layers", layers),
        ];
        top.sort_by(|a, b| a.0.cmp(b.0));
        encode(&BValue::Dict(top))
    }

    /// Single-file pure-v2 torrent with an explicit piece length. Piece layers
    /// are emitted only when the file spans more than one piece. Use a
    /// power-of-two piece count so the layer root needs no padding rules.
    fn v2_raw_with_piece_length(content: &[u8], piece_length: usize) -> Vec<u8> {
        use rt_bencode::{encode, BValue};
        use rt_hash::{merkle_root, piece_layer_hash};
        let layered = content.len() > piece_length;
        let piece_layers: Vec<[u8; 32]> = content
            .chunks(piece_length)
            .map(|chunk| piece_layer_hash(chunk, piece_length).unwrap())
            .collect();
        let pieces_root = if layered {
            merkle_root(&piece_layers)
        } else {
            merkle_root(&v2_leaf_hashes(content))
        };
        let mut layer_bytes = Vec::new();
        for hash in &piece_layers {
            layer_bytes.extend_from_slice(hash);
        }
        let leaf = BValue::Dict(vec![
            (b"length".as_ref(), BValue::Int(content.len() as i64)),
            (b"pieces root".as_ref(), BValue::Bytes(&pieces_root)),
        ]);
        let file_tree = BValue::Dict(vec![(
            b"data.bin".as_ref(),
            BValue::Dict(vec![(b"".as_ref(), leaf)]),
        )]);
        let mut info: Vec<(&[u8], BValue<'_>)> = vec![
            (b"file tree", file_tree),
            (b"meta version", BValue::Int(2)),
            (b"name", BValue::Bytes(b"v2dir")),
            (b"piece length", BValue::Int(piece_length as i64)),
        ];
        info.sort_by(|a, b| a.0.cmp(b.0));
        let mut top: Vec<(&[u8], BValue<'_>)> = vec![
            (b"announce", BValue::Bytes(b"http://tracker.example/v2")),
            (b"info", BValue::Dict(info)),
        ];
        if layered {
            top.push((
                b"piece layers",
                BValue::Dict(vec![(&pieces_root[..], BValue::Bytes(&layer_bytes))]),
            ));
        }
        top.sort_by(|a, b| a.0.cmp(b.0));
        encode(&BValue::Dict(top))
    }

    struct V2Fixture {
        temp: tempfile::TempDir,
        registry: Arc<RwLock<SessionRegistry>>,
        info_hash: String,
    }

    async fn v2_task(runtime: Arc<CrashSafetyRuntime>) -> (V2TorrentTask, V2Fixture) {
        let raw = v2_raw_with_content(&vec![0x5Au8; 32_768]);
        let (task, fx) = v2_task_not_started(runtime, &raw, 32_768).await;
        // Every piece is on disk and verified: the download just finished.
        let mut have = V2Bitmap::new(task.piece_map.piece_count as usize);
        for piece in 0..task.piece_map.piece_count as usize {
            have.set(piece, true);
        }
        *task.local_have.write().await = Arc::new(have);
        (task, fx)
    }

    /// A downloading task for `raw` with nothing yet on disk or in the bitmap.
    async fn v2_task_not_started(
        runtime: Arc<CrashSafetyRuntime>,
        raw: &[u8],
        total_length: u64,
    ) -> (V2TorrentTask, V2Fixture) {
        let temp = tempfile::tempdir().unwrap();
        let raw = raw.to_vec();
        let rt_metainfo::TorrentMeta::V2(meta) = rt_metainfo::parse_torrent(&raw).unwrap() else {
            panic!("expected a pure-v2 torrent");
        };
        let info_hash = hex::encode(meta.info_hash_v2);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        rt_db::migrate(&conn).unwrap();
        let mut entry = rt_session::TorrentEntry::new(
            info_hash.clone(),
            meta.name.clone(),
            temp.path().to_string_lossy().into_owned(),
        );
        entry.total_length = total_length;
        entry.amount_left = total_length;
        entry.state = TorrentState::Downloading;
        rt_db::upsert(&conn, &crate::engine::row_from_v2_meta(&entry, &meta)).unwrap();
        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        registry.write().await.add(entry).unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        std::mem::forget(cmd_tx);
        let mut task = V2TorrentTask::new(
            meta,
            temp.path().to_path_buf(),
            false,
            TorrentState::Downloading,
            Arc::clone(&registry),
            DbExecutor::direct(Arc::new(std::sync::Mutex::new(conn))),
            ResourceGovernor::new(rt_metrics::ResourceGovernorConfig::default()),
            cmd_rx,
            8,
            1024 * 1024,
            StorageIoConfig::default(),
            OutboundEgressPolicy::default(),
            6881,
            10,
            10,
            60,
            GlobalNetworkBudget::unlimited(),
            None,
            None,
        )
        .await;
        task.attach_crash_safety(runtime);
        (
            task,
            V2Fixture {
                temp,
                registry,
                info_hash,
            },
        )
    }

    /// A genuine I/O failure (EIO on Unix, ERROR_CRC on Windows), not an
    /// "unsupported" answer.
    fn eio() -> rt_storage::StorageError {
        rt_storage::StorageError::Io {
            path: "payload".to_owned(),
            source: std::io::Error::from_raw_os_error(if cfg!(windows) { 23 } else { 5 }),
        }
    }

    fn quiet_v2_runtime() -> Arc<CrashSafetyRuntime> {
        // Weak-mount escalation is irrelevant to this task type, and /tmp is
        // often tmpfs; keep the runtime independent of the temp filesystem.
        let runtime = CrashSafetyRuntime::inert();
        runtime.apply_settings(
            rt_config::CrashSafetyConfig {
                weak_mount_escalation: false,
                ..rt_config::CrashSafetyConfig::default()
            },
            false,
        );
        runtime
    }

    #[tokio::test]
    async fn v2_failed_barrier_holds_completion_and_reports_one_byte() {
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        assert!(task.is_complete().await);

        // The barrier fails: not releasable, gauge raised once.
        assert!(!task.apply_barrier_result(Err(eio())));
        assert!(
            !task.apply_barrier_result(Err(eio())),
            "a retry is still held"
        );
        assert_eq!(runtime.counters().completions_pending, 1, "counted once");
        assert_eq!(runtime.counters().completions_gated, 1);

        // A held torrent is complete on disk but must not look complete.
        task.persist_runtime(None, None).await.unwrap();
        let entry = fx.registry.read().await.get(&fx.info_hash).unwrap();
        assert_eq!(entry.state, TorrentState::Downloading);
        assert_eq!(entry.amount_left, 1);
        assert!(entry.finalizing);
        assert_eq!(entry.completed_at, None);

        // Storage recovers: released, becomes Seeding with true accounting.
        assert!(task.apply_barrier_result(Ok(())));
        assert_eq!(runtime.counters().completions_pending, 0);
        assert_eq!(runtime.counters().completions_released, 1);
        task.persist_runtime(Some(TorrentState::Seeding), None)
            .await
            .unwrap();
        let entry = fx.registry.read().await.get(&fx.info_hash).unwrap();
        assert_eq!(entry.state, TorrentState::Seeding);
        assert_eq!(entry.amount_left, 0);
        assert!(!entry.finalizing);
        assert!(entry.completed_at.is_some());
    }

    #[tokio::test]
    async fn v2_filesystem_that_cannot_fsync_is_released_and_rated_weak() {
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        // Hold it first so the unsupported answer must also clear the hold.
        assert!(!task.apply_barrier_result(Err(eio())));
        let unsupported = rt_storage::StorageError::Io {
            path: "payload".to_owned(),
            // ENOSYS on Unix, ERROR_NOT_SUPPORTED on Windows.
            source: std::io::Error::from_raw_os_error(if cfg!(windows) { 50 } else { 38 }),
        };
        assert!(task.apply_barrier_result(Err(unsupported)));
        assert!(!task
            .completion_held
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(task
            .fsync_unsupported
            .load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(runtime.counters().completions_pending, 0);
        assert_eq!(runtime.counters().completion_sync_unsupported, 1);
        assert_eq!(
            runtime.mount_durability(fx.temp.path()).trust,
            rt_storage::DurabilityTrust::Weak
        );
        // Remembered: later completions skip the doomed sync entirely.
        assert!(task.completion_may_release().await);
    }

    #[tokio::test]
    async fn v2_gate_can_be_disabled() {
        let runtime = quiet_v2_runtime();
        runtime.apply_settings(
            rt_config::CrashSafetyConfig {
                completion_gate: false,
                weak_mount_escalation: false,
                ..rt_config::CrashSafetyConfig::default()
            },
            true,
        );
        let (task, _fx) = v2_task(Arc::clone(&runtime)).await;
        assert!(task.completion_may_release().await);
        assert_eq!(runtime.counters().completions_gated, 0);
    }

    #[tokio::test]
    async fn v2_location_policy_applies_only_to_matching_save_roots() {
        // The gate is on globally; the location that holds this torrent opts out.
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        let with_policies = |path: &std::path::Path| rt_config::CrashSafetyConfig {
            weak_mount_escalation: false,
            path_policies: vec![rt_config::PathPolicy {
                path: path.to_path_buf(),
                completion_gate: Some(false),
                ..rt_config::PathPolicy::default()
            }],
            ..rt_config::CrashSafetyConfig::default()
        };
        runtime.apply_settings(with_policies(fx.temp.path()), true);
        assert!(task.completion_may_release().await);
        assert_eq!(runtime.counters().completions_gated, 0, "gate skipped here");

        // The same policy pointed elsewhere leaves this torrent gated: a
        // failing barrier holds it.
        runtime.apply_settings(with_policies(std::path::Path::new("/srv/elsewhere")), true);
        assert!(!task.apply_barrier_result(Err(eio())));
        assert_eq!(runtime.counters().completions_gated, 1);
    }

    #[tokio::test]
    async fn v2_location_policy_can_require_read_back() {
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        runtime.apply_settings(
            rt_config::CrashSafetyConfig {
                weak_mount_escalation: false,
                completion_verify: rt_config::CompletionVerifyMode::Off,
                path_policies: vec![rt_config::PathPolicy {
                    path: fx.temp.path().to_path_buf(),
                    completion_verify: Some(rt_config::CompletionVerifyMode::Full),
                    ..rt_config::PathPolicy::default()
                }],
                ..rt_config::CrashSafetyConfig::default()
            },
            true,
        );
        // Bytes on disk differ from the metainfo: only a read-back notices.
        let mut on_disk = vec![0x5Au8; 32_768];
        on_disk[..16_384].fill(0);
        v2_write_payload(&task, &fx, &[("data.bin", on_disk)]);
        assert!(!task.completion_may_release().await);
        assert!(v2_event_kinds(&task).contains(&"completion_verify_failed".to_owned()));
    }

    #[tokio::test]
    async fn v2_release_with_a_working_disk_needs_no_hold() {
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        assert!(task.completion_may_release().await, "real sync succeeds");
        task.persist_runtime(Some(TorrentState::Seeding), None)
            .await
            .unwrap();
        let entry = fx.registry.read().await.get(&fx.info_hash).unwrap();
        assert_eq!(entry.state, TorrentState::Seeding);
        assert_eq!(entry.amount_left, 0);
        assert_eq!(runtime.counters().completions_gated, 0, "never held");
    }

    // ---- crash safety: read-back verification ------------------------------

    /// A pure-v2 torrent of several small files (each at most one 16 KiB
    /// piece, so no piece layers are needed).
    fn v2_raw_multi(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use rt_bencode::{encode, BValue};
        use rt_hash::BlockHash;
        let mut roots: Vec<[u8; 32]> = Vec::new();
        for (_, content) in files {
            assert!(content.len() <= 16_384);
            roots.push(BlockHash::of(content).0);
        }
        let mut tree: Vec<(&[u8], BValue<'_>)> = Vec::new();
        for (index, (name, content)) in files.iter().enumerate() {
            let leaf = BValue::Dict(vec![
                (b"length".as_ref(), BValue::Int(content.len() as i64)),
                (b"pieces root".as_ref(), BValue::Bytes(&roots[index])),
            ]);
            tree.push((name.as_bytes(), BValue::Dict(vec![(b"".as_ref(), leaf)])));
        }
        tree.sort_by(|a, b| a.0.cmp(b.0));
        let mut info: Vec<(&[u8], BValue<'_>)> = vec![
            (b"file tree", BValue::Dict(tree)),
            (b"meta version", BValue::Int(2)),
            (b"name", BValue::Bytes(b"multi")),
            (b"piece length", BValue::Int(16_384)),
        ];
        info.sort_by(|a, b| a.0.cmp(b.0));
        let mut top: Vec<(&[u8], BValue<'_>)> = vec![
            (b"announce", BValue::Bytes(b"http://tracker.example/v2")),
            (b"info", BValue::Dict(info)),
            (b"piece layers", BValue::Dict(vec![])),
        ];
        top.sort_by(|a, b| a.0.cmp(b.0));
        encode(&BValue::Dict(top))
    }

    async fn v2_task_from_raw(
        runtime: Arc<CrashSafetyRuntime>,
        raw: &[u8],
    ) -> (V2TorrentTask, V2Fixture) {
        let temp = tempfile::tempdir().unwrap();
        let rt_metainfo::TorrentMeta::V2(meta) = rt_metainfo::parse_torrent(raw).unwrap() else {
            panic!("expected a pure-v2 torrent");
        };
        let info_hash = hex::encode(meta.info_hash_v2);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        rt_db::migrate(&conn).unwrap();
        let total = meta.files.iter().map(|file| file.length).sum::<u64>();
        let mut entry = rt_session::TorrentEntry::new(
            info_hash.clone(),
            meta.name.clone(),
            temp.path().to_string_lossy().into_owned(),
        );
        entry.total_length = total;
        entry.amount_left = total;
        entry.state = TorrentState::Downloading;
        rt_db::upsert(&conn, &crate::engine::row_from_v2_meta(&entry, &meta)).unwrap();
        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        registry.write().await.add(entry).unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        std::mem::forget(cmd_tx);
        let mut task = V2TorrentTask::new(
            meta,
            temp.path().to_path_buf(),
            false,
            TorrentState::Downloading,
            Arc::clone(&registry),
            DbExecutor::direct(Arc::new(std::sync::Mutex::new(conn))),
            ResourceGovernor::new(rt_metrics::ResourceGovernorConfig::default()),
            cmd_rx,
            8,
            1024 * 1024,
            StorageIoConfig::default(),
            OutboundEgressPolicy::default(),
            6881,
            10,
            10,
            60,
            GlobalNetworkBudget::unlimited(),
            None,
            None,
        )
        .await;
        task.attach_crash_safety(runtime);
        let mut have = V2Bitmap::new(task.piece_map.piece_count as usize);
        for piece in 0..task.piece_map.piece_count as usize {
            have.set(piece, true);
        }
        *task.local_have.write().await = Arc::new(have);
        (
            task,
            V2Fixture {
                temp,
                registry,
                info_hash,
            },
        )
    }

    fn v2_write_payload(task: &V2TorrentTask, fx: &V2Fixture, contents: &[(&str, Vec<u8>)]) {
        for file in &task.meta.files {
            let name = file.path.components().last().cloned().unwrap_or_default();
            let content = contents
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("no content for {name}"));
            let path = file.path.resolve(fx.temp.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &content.1).unwrap();
        }
    }

    fn v2_verify_runtime(
        mode: rt_config::CompletionVerifyMode,
        percent: u8,
    ) -> Arc<CrashSafetyRuntime> {
        let runtime = CrashSafetyRuntime::inert();
        runtime.apply_settings(
            rt_config::CrashSafetyConfig {
                weak_mount_escalation: false,
                completion_verify: mode,
                completion_verify_sample_percent: percent,
                ..rt_config::CrashSafetyConfig::default()
            },
            true,
        );
        runtime
    }

    fn v2_event_kinds(task: &V2TorrentTask) -> Vec<String> {
        // The executor is `direct` over an in-memory connection.
        let mut kinds = Vec::new();
        let rows = futures::executor::block_on(task.db.run("read_events", |db| {
            let mut stmt = db
                .prepare("SELECT kind FROM session_events ORDER BY event_id")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        }));
        kinds.extend(rows.unwrap_or_default());
        kinds
    }

    #[test]
    fn v2_sample_selection_is_deterministic_covers_the_budget_and_never_empty() {
        let files: Vec<(u32, u64)> = (0..10).map(|i| (i, 1000)).collect();
        let a = select_v2_sample_files(&files, 30, 7).unwrap();
        assert_eq!(
            a,
            select_v2_sample_files(&files, 30, 7).unwrap(),
            "same seed"
        );
        assert_ne!(
            a,
            select_v2_sample_files(&files, 30, 8).unwrap(),
            "seed matters"
        );
        assert_eq!(a.len(), 3, "30% of ten equal files");
        assert!(a.windows(2).all(|w| w[0] < w[1]), "sorted and distinct");

        // A tiny percentage still checks something.
        assert_eq!(select_v2_sample_files(&files, 1, 7).unwrap().len(), 1);
        // 100%, or a budget that needs every file, is just a full check.
        assert_eq!(select_v2_sample_files(&files, 100, 7), None);
        assert_eq!(select_v2_sample_files(&files, 99, 7), None);
        // One file cannot be sampled below itself.
        assert_eq!(select_v2_sample_files(&[(0, 5000)], 5, 7), None);
        assert_eq!(select_v2_sample_files(&[], 5, 7), None);
    }

    #[test]
    fn v2_sample_selection_stops_once_the_byte_budget_is_met() {
        // One huge file and many tiny ones: the budget is bytes, not files.
        let mut files: Vec<(u32, u64)> = (1..=20).map(|i| (i, 10)).collect();
        files.push((0, 10_000));
        let total: u64 = files.iter().map(|f| f.1).sum();
        let picked = select_v2_sample_files(&files, 5, 3).unwrap();
        let covered: u64 = picked
            .iter()
            .map(|i| files.iter().find(|f| f.0 == *i).unwrap().1)
            .sum();
        assert!(covered * 100 >= total * 5, "budget met: {covered}/{total}");
        assert!(picked.len() < files.len());
    }

    #[tokio::test]
    async fn v2_full_verification_releases_a_clean_download() {
        let runtime = v2_verify_runtime(rt_config::CompletionVerifyMode::Full, 5);
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        let content = vec![0x5Au8; 32_768];
        v2_write_payload(&task, &fx, &[("data.bin", content)]);

        assert!(task.completion_may_release().await);
        assert!(!task
            .completion_held
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(v2_event_kinds(&task).contains(&"completion_verified".to_owned()));
        let counters = runtime.counters();
        assert_eq!(counters.completion_verify_failures, 0);
        assert!(counters.completion_verify_pieces >= 1);
        assert_eq!(counters.completions_pending, 0);
        assert_eq!(counters.completions_released, 1);
    }

    #[tokio::test]
    async fn v2_verification_failure_reopens_the_download_and_clears_the_hold() {
        let runtime = v2_verify_runtime(rt_config::CompletionVerifyMode::Full, 5);
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        // The first 16 KiB reached the page cache but never the disk.
        let mut on_disk = vec![0x5Au8; 32_768];
        on_disk[..16_384].fill(0);
        v2_write_payload(&task, &fx, &[("data.bin", on_disk)]);

        assert!(!task.completion_may_release().await, "must not be released");
        assert!(!task.is_complete().await, "the bad pieces are wanted again");
        assert!(!task
            .completion_held
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(v2_event_kinds(&task).contains(&"completion_verify_failed".to_owned()));
        assert_eq!(runtime.counters().completion_verify_failures, 2);
        assert_eq!(runtime.counters().completions_pending, 0, "gauge released");

        task.persist_runtime(None, None).await.unwrap();
        let entry = fx.registry.read().await.get(&fx.info_hash).unwrap();
        assert_eq!(entry.state, TorrentState::Downloading);
        assert!(!entry.finalizing);
        assert_eq!(entry.amount_left, 32_768);
    }

    #[tokio::test]
    async fn v2_sample_misses_damage_outside_it_but_a_hit_escalates_to_a_full_check() {
        let names = ["a.bin", "b.bin", "c.bin", "d.bin"];
        let good: Vec<(&str, Vec<u8>)> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, vec![i as u8 + 1; 16_384]))
            .collect();
        let raw = v2_raw_multi(&good);

        // Which single file does a 25% sample pick for this torrent?
        let runtime = v2_verify_runtime(rt_config::CompletionVerifyMode::Sample, 25);
        let (probe, _fx) = v2_task_from_raw(Arc::clone(&runtime), &raw).await;
        let sizes: Vec<(u32, u64)> = probe
            .meta
            .files
            .iter()
            .map(|f| (f.index, f.length))
            .collect();
        let mut seed = [0u8; 8];
        for (slot, byte) in seed.iter_mut().zip(probe.meta.info_hash_v2.iter()) {
            *slot = *byte;
        }
        let sampled = select_v2_sample_files(&sizes, 25, u64::from_le_bytes(seed)).unwrap();
        assert_eq!(sampled.len(), 1);
        let sampled_name = |task: &V2TorrentTask| {
            task.meta
                .files
                .iter()
                .find(|f| f.index == sampled[0])
                .unwrap()
                .path
                .components()
                .last()
                .cloned()
                .unwrap()
        };
        let sampled_file = sampled_name(&probe);
        let other_file = names
            .iter()
            .find(|n| **n != sampled_file)
            .map(|n| n.to_string())
            .unwrap();
        let corrupt = |broken: &[&str]| -> Vec<(&str, Vec<u8>)> {
            good.iter()
                .map(|(n, c)| {
                    let mut c = c.clone();
                    if broken.contains(n) {
                        c.fill(0xEE);
                    }
                    (*n, c)
                })
                .collect()
        };

        // Damage the sample does not cover: it passes (sampling is a spot
        // check, and the docs say so).
        let runtime = v2_verify_runtime(rt_config::CompletionVerifyMode::Sample, 25);
        let (task, fx) = v2_task_from_raw(Arc::clone(&runtime), &raw).await;
        v2_write_payload(&task, &fx, &corrupt(&[other_file.as_str()]));
        assert!(
            task.completion_may_release().await,
            "damage outside the sample"
        );
        assert_eq!(runtime.counters().completion_verify_failures, 0);

        // Damage the sample covers, plus another file: the failed sample
        // escalates and the full check finds both.
        let runtime = v2_verify_runtime(rt_config::CompletionVerifyMode::Sample, 25);
        let (task, fx) = v2_task_from_raw(Arc::clone(&runtime), &raw).await;
        v2_write_payload(
            &task,
            &fx,
            &corrupt(&[sampled_file.as_str(), other_file.as_str()]),
        );
        assert!(!task.completion_may_release().await);
        assert_eq!(
            runtime.counters().completion_verify_failures,
            2,
            "escalation to a full check found the second damaged file"
        );
        assert!(!task.is_complete().await);
    }

    #[tokio::test]
    async fn v2_weak_mounts_get_a_read_back_even_when_verification_is_off() {
        let runtime = CrashSafetyRuntime::inert();
        runtime.apply_settings(
            rt_config::CrashSafetyConfig {
                weak_mount_escalation: true,
                ..rt_config::CrashSafetyConfig::default()
            },
            true,
        );
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        // Declare the save path weak regardless of the temp filesystem.
        let mut settings = (*runtime.settings()).clone();
        settings.weak_mount_paths = vec![fx.temp.path().to_path_buf()];
        runtime.apply_settings(settings, true);
        // Nothing on disk: a read-back must catch that.
        assert!(!task.completion_may_release().await);
        assert!(v2_event_kinds(&task).contains(&"completion_verify_failed".to_owned()));
    }

    #[tokio::test]
    async fn v2_recheck_of_a_completed_torrent_reports_an_integrity_regression() {
        let runtime = quiet_v2_runtime();
        let (task, fx) = v2_task(Arc::clone(&runtime)).await;
        {
            let mut registry = fx.registry.write().await;
            let mut entry = registry.get_mut(&fx.info_hash).unwrap();
            entry.completed_at = Some(1_700_000_000);
            entry.state = TorrentState::Seeding;
        }
        // Only the first of the two pieces is intact.
        let mut have = V2Bitmap::new(task.piece_map.piece_count as usize);
        have.set(0, true);
        task.report_v2_recheck(&have, "recheck").await;
        assert_eq!(runtime.counters().integrity_regressions, 1);
        assert!(v2_event_kinds(&task).contains(&"integrity_regression".to_owned()));

        // A torrent never reported complete is not a regression.
        let runtime = quiet_v2_runtime();
        let (fresh, _fx) = v2_task(Arc::clone(&runtime)).await;
        fresh.report_v2_recheck(&have, "startup").await;
        assert_eq!(runtime.counters().integrity_regressions, 0);
    }

    // ---- multi-block pieces ------------------------------------------------
    //
    // The fixtures above use 16 KiB pieces, where every piece is one block and
    // a block extent equals a piece extent. These use 32 KiB pieces so the two
    // differ, with a sentinel-filled payload so a misplaced write is visible.

    const MB_BLOCK: usize = 16_384;
    const MB_PIECE: usize = 32_768;
    const MB_SENTINEL: u8 = 0xEE;

    /// Distinct bytes in every block, never equal to the sentinel run.
    fn mb_content(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                let byte = ((i / MB_BLOCK) as u8)
                    .wrapping_mul(31)
                    .wrapping_add(i as u8);
                if byte == MB_SENTINEL {
                    0
                } else {
                    byte
                }
            })
            .collect()
    }

    struct MultiBlock {
        task: V2TorrentTask,
        fx: V2Fixture,
        content: Vec<u8>,
    }

    impl MultiBlock {
        async fn new(len: usize, spill_to_disk: bool) -> Self {
            let content = mb_content(len);
            let raw = v2_raw_with_piece_length(&content, MB_PIECE);
            let (mut task, fx) = v2_task_not_started(quiet_v2_runtime(), &raw, len as u64).await;
            if spill_to_disk {
                // A zero cap sends every block straight to the file.
                task.assembly_cap_bytes = 0;
            }
            let path = task.meta.files[0].path.resolve(fx.temp.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![MB_SENTINEL; len]).unwrap();
            Self { task, fx, content }
        }

        fn piece_len(&self, piece: usize) -> usize {
            (self.content.len() - piece * MB_PIECE).min(MB_PIECE)
        }

        fn block(&self, piece: usize, block: usize) -> (u32, Bytes) {
            let begin = block * MB_BLOCK;
            let len = (self.piece_len(piece) - begin).min(MB_BLOCK);
            let start = piece * MB_PIECE + begin;
            (
                begin as u32,
                Bytes::copy_from_slice(&self.content[start..start + len]),
            )
        }

        /// Deliver `order` (block indexes within `piece`) through the real
        /// handler. Returns the peer state so progress can be checked.
        async fn deliver(
            &self,
            piece: usize,
            order: &[usize],
        ) -> anyhow::Result<Arc<Mutex<V2PeerState>>> {
            let context = self
                .task
                .peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
            let state = Arc::new(Mutex::new(V2PeerState::new(
                self.task.piece_map.piece_count as usize,
            )));
            let mut outstanding = HashMap::new();
            let mut assemblies = HashMap::new();
            let mut pending_piece_completions = HashMap::new();
            let mut pending_block_completions = HashMap::new();
            let mut transfer_jobs = futures::stream::FuturesUnordered::<V2TransferTask>::new();
            for &block in order {
                let (begin, data) = self.block(piece, block);
                outstanding.insert(
                    (piece as u32, begin),
                    V2OutstandingRequest {
                        length: data.len() as u32,
                        _memory_lease: None,
                    },
                );
            }
            for &block in order {
                let (begin, data) = self.block(piece, block);
                handle_v2_piece(
                    &context,
                    &state,
                    &mut outstanding,
                    &mut assemblies,
                    &mut transfer_jobs,
                    &mut pending_block_completions,
                    piece as u32,
                    begin,
                    data,
                )
                .await?;
                start_ready_v2_piece_jobs(
                    &context,
                    &mut assemblies,
                    &mut pending_piece_completions,
                    &mut transfer_jobs,
                );
            }
            drain_v2_transfer_jobs(
                &context,
                &state,
                &mut assemblies,
                &mut pending_block_completions,
                &mut pending_piece_completions,
                &mut transfer_jobs,
            )
            .await?;
            Ok(state)
        }

        async fn have(&self, piece: usize) -> bool {
            self.task.local_have.read().await.get(piece)
        }

        /// Bytes on disk equal the content for `done` pieces and are still
        /// untouched sentinel everywhere else.
        fn assert_payload(&self, done: &[usize]) {
            let path = self.task.meta.files[0].path.resolve(self.fx.temp.path());
            let disk = std::fs::read(path).unwrap();
            assert_eq!(disk.len(), self.content.len());
            for piece in 0..self.content.len().div_ceil(MB_PIECE) {
                let range = piece * MB_PIECE..piece * MB_PIECE + self.piece_len(piece);
                if done.contains(&piece) {
                    assert!(
                        disk[range.clone()] == self.content[range],
                        "piece {piece} content"
                    );
                } else {
                    assert!(
                        disk[range].iter().all(|byte| *byte == MB_SENTINEL),
                        "piece {piece} must be untouched"
                    );
                }
            }
        }
    }

    /// Run one delivery order in both assembly modes and check every
    /// observable of a completed piece.
    async fn assert_multi_block_piece(len: usize, piece: usize, order: &[usize]) {
        for spill_to_disk in [false, true] {
            let mut case = MultiBlock::new(len, spill_to_disk).await;
            let label = format!("len={len} piece={piece} order={order:?} spill={spill_to_disk}");
            let state = case
                .deliver(piece, order)
                .await
                .unwrap_or_else(|error| panic!("{label}: {error:#}"));
            assert!(case.have(piece).await, "{label}: piece marked have");
            for other in 0..len.div_ceil(MB_PIECE) {
                if other != piece {
                    assert!(!case.have(other).await, "{label}: piece {other} untouched");
                }
            }
            case.assert_payload(&[piece]);
            let piece_len = case.piece_len(piece) as u64;
            assert_eq!(state.lock().unwrap().downloaded, piece_len, "{label}");
            match case.task.events_rx.try_recv() {
                Ok(V2PeerEvent::Downloaded { bytes, .. }) => {
                    assert_eq!(bytes, piece_len, "{label}: whole-piece progress");
                }
                other => panic!("{label}: expected a Downloaded event, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn v2_piece_is_not_available_until_completion_job_finishes() {
        let case = MultiBlock::new(MB_PIECE, false).await;
        let context = case
            .task
            .peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
        let state = Arc::new(Mutex::new(V2PeerState::new(
            case.task.piece_map.piece_count as usize,
        )));
        let mut outstanding = HashMap::new();
        let mut assemblies = HashMap::new();
        let mut pending_piece_completions = HashMap::new();
        let mut pending_block_completions = HashMap::new();
        let mut transfer_jobs = futures::stream::FuturesUnordered::<V2TransferTask>::new();

        for block in 0..MB_PIECE.div_ceil(MB_BLOCK) {
            let (begin, data) = case.block(0, block);
            outstanding.insert(
                (0, begin),
                V2OutstandingRequest {
                    length: data.len() as u32,
                    _memory_lease: None,
                },
            );
            handle_v2_piece(
                &context,
                &state,
                &mut outstanding,
                &mut assemblies,
                &mut transfer_jobs,
                &mut pending_block_completions,
                0,
                begin,
                data,
            )
            .await
            .unwrap();
            start_ready_v2_piece_jobs(
                &context,
                &mut assemblies,
                &mut pending_piece_completions,
                &mut transfer_jobs,
            );
        }

        while !pending_block_completions.is_empty() {
            let completion = transfer_jobs
                .next()
                .await
                .expect("queued block-admission work");
            let V2TransferCompletion::BlockStored(completion) = completion else {
                panic!("piece verification cannot precede block admission");
            };
            apply_v2_block_completion(completion, &mut assemblies, &mut pending_block_completions)
                .unwrap();
            start_ready_v2_piece_jobs(
                &context,
                &mut assemblies,
                &mut pending_piece_completions,
                &mut transfer_jobs,
            );
        }
        assert_eq!(transfer_jobs.len(), 1);
        assert!(!case.have(0).await, "queued verification is not completion");
        assert!(case.task.events_rx.try_recv().is_err());
        case.assert_payload(&[]);

        let Some(V2TransferCompletion::PieceVerified(completion)) = transfer_jobs.next().await
        else {
            panic!("piece worker is queued");
        };
        apply_v2_piece_completion(&context, &state, &mut pending_piece_completions, completion)
            .await;

        assert!(case.have(0).await);
        case.assert_payload(&[0]);
        assert!(matches!(
            case.task.events_rx.try_recv(),
            Ok(V2PeerEvent::Downloaded { bytes, .. }) if bytes == MB_PIECE as u64
        ));
    }

    #[tokio::test]
    async fn v2_two_block_piece_completes_in_order() {
        assert_multi_block_piece(4 * MB_PIECE, 0, &[0, 1]).await;
    }

    #[tokio::test]
    async fn v2_piece_at_nonzero_offset_writes_only_its_own_extent() {
        assert_multi_block_piece(4 * MB_PIECE, 2, &[0, 1]).await;
    }

    #[tokio::test]
    async fn v2_piece_completes_when_its_first_block_arrives_last() {
        assert_multi_block_piece(4 * MB_PIECE, 1, &[1, 0]).await;
    }

    #[tokio::test]
    async fn v2_short_final_piece_with_a_short_final_block_completes() {
        // The last piece is 20,000 bytes: one full block and a 3,616-byte tail.
        let len = 3 * MB_PIECE + 20_000;
        assert_multi_block_piece(len, 3, &[0, 1]).await;
        assert_multi_block_piece(len, 3, &[1, 0]).await;
    }

    #[tokio::test]
    async fn v2_partial_piece_is_not_completed_or_counted() {
        for spill_to_disk in [false, true] {
            let mut case = MultiBlock::new(4 * MB_PIECE, spill_to_disk).await;
            let state = case.deliver(1, &[0]).await.unwrap();
            assert!(!case.have(1).await, "spill={spill_to_disk}");
            assert_eq!(state.lock().unwrap().downloaded, 0);
            assert!(case.task.events_rx.try_recv().is_err());
            if spill_to_disk {
                // Only the first block has landed, at its own offset.
                let path = case.task.meta.files[0].path.resolve(case.fx.temp.path());
                let disk = std::fs::read(path).unwrap();
                assert_eq!(
                    disk[MB_PIECE..MB_PIECE + MB_BLOCK],
                    case.content[MB_PIECE..MB_PIECE + MB_BLOCK]
                );
                assert!(disk[MB_PIECE + MB_BLOCK..2 * MB_PIECE]
                    .iter()
                    .all(|byte| *byte == MB_SENTINEL));
            }
        }
    }

    #[tokio::test]
    async fn v2_request_window_pipelines_every_block_of_a_piece() {
        // 4 pieces of 2 blocks each: a 32-request window must hold all 8
        // blocks, not one per piece.
        let content = mb_content(4 * MB_PIECE);
        let raw = v2_raw_with_piece_length(&content, MB_PIECE);
        let (task, _fx) = v2_task_not_started(quiet_v2_runtime(), &raw, content.len() as u64).await;
        let context = task.peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
        let piece_count = task.piece_map.piece_count as usize;
        let mut remote = V2Bitmap::new(piece_count);
        for piece in 0..piece_count {
            remote.set(piece, true);
        }
        let mut peer = V2PeerState::new(piece_count);
        peer.remote_have = Arc::new(remote);
        peer.choked = false;
        let state = Arc::new(Mutex::new(peer));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap());
        let (client, accepted) = tokio::join!(client, listener.accept());
        let _server = accepted.unwrap();
        let mut framed = PeerIo::Tcp(Framed::with_capacity(
            client.unwrap(),
            PeerCodec::with_resources(context.resources.clone()),
            1,
        ));

        let mut outstanding = HashMap::new();
        let mut assemblies = HashMap::new();
        let pending_block_completions = HashMap::new();
        let pending_piece_completions = HashMap::new();
        fill_v2_requests(
            &context,
            &mut framed,
            &state,
            &mut outstanding,
            &mut assemblies,
            0,
            &pending_block_completions,
            &pending_piece_completions,
        )
        .await
        .unwrap();
        assert_eq!(outstanding.len(), 8, "{outstanding:?}");
        for piece in 0..piece_count as u32 {
            for begin in [0, MB_BLOCK as u32] {
                assert!(
                    outstanding.contains_key(&(piece, begin)),
                    "({piece},{begin})"
                );
            }
        }
    }

    #[tokio::test]
    async fn v2_request_admission_retries_after_capacity_returns_and_owns_each_block() {
        let content = mb_content(2 * MB_PIECE);
        let raw = v2_raw_with_piece_length(&content, MB_PIECE);
        let (task, _fx) = v2_task_not_started(quiet_v2_runtime(), &raw, content.len() as u64).await;
        let context = task.peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
        let resources = context.resources.clone();
        let piece_count = task.piece_map.piece_count as usize;
        let mut remote_have = V2Bitmap::new(piece_count);
        for piece in 0..piece_count {
            remote_have.set(piece, true);
        }
        let mut peer = V2PeerState::new(piece_count);
        peer.remote_have = Arc::new(remote_have);
        peer.choked = false;
        let state = Arc::new(Mutex::new(peer));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap());
        let (client, accepted) = tokio::join!(client, listener.accept());
        let mut framed = PeerIo::Tcp(Framed::with_capacity(
            client.unwrap(),
            PeerCodec::default(),
            1,
        ));
        let mut remote = Framed::new(accepted.unwrap().0, PeerCodec::default());
        let mut outstanding = HashMap::new();
        let mut assemblies = HashMap::new();
        let mut pending_block_completions = HashMap::new();
        let mut pending_piece_completions = HashMap::new();
        let mut transfer_jobs = futures::stream::FuturesUnordered::<V2TransferTask>::new();
        let peer_buffer_index = MemoryClass::PeerBuffer as usize;
        let baseline_used = resources.snapshot().classes[peer_buffer_index].used_bytes;
        let snapshot = resources.snapshot();
        let class = snapshot.classes[peer_buffer_index];
        let class_available = class.cap_bytes.saturating_sub(class.used_bytes);
        let total_available = snapshot
            .total_cap_bytes
            .saturating_sub(snapshot.total_used_bytes);
        let blocker_bytes = class_available.min(total_available);
        assert!(blocker_bytes > 0);
        let blocker = resources
            .try_acquire(MemoryClass::PeerBuffer, blocker_bytes)
            .expect("test should hold the remaining peer-buffer admission");
        let denied_before = resources.snapshot().classes[peer_buffer_index].denied_allocations;

        fill_v2_requests(
            &context,
            &mut framed,
            &state,
            &mut outstanding,
            &mut assemblies,
            transfer_jobs.len(),
            &pending_block_completions,
            &pending_piece_completions,
        )
        .await
        .unwrap();

        assert!(outstanding.is_empty());
        assert!(assemblies.is_empty());
        assert_eq!(
            resources.snapshot().classes[peer_buffer_index].denied_allocations,
            denied_before + 1
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), remote.next())
                .await
                .is_err(),
            "a denied v2 request must not reach the wire"
        );

        drop(blocker);
        fill_v2_requests(
            &context,
            &mut framed,
            &state,
            &mut outstanding,
            &mut assemblies,
            transfer_jobs.len(),
            &pending_block_completions,
            &pending_piece_completions,
        )
        .await
        .unwrap();
        assert_eq!(outstanding.len(), 4);
        for (piece, begin) in [(0, 0), (0, MB_BLOCK as u32), (1, 0), (1, MB_BLOCK as u32)] {
            assert!(outstanding.contains_key(&(piece, begin)));
            let message = tokio::time::timeout(Duration::from_secs(1), remote.next())
                .await
                .expect("admitted request should reach the peer")
                .expect("peer socket should remain open")
                .expect("request frame should decode");
            match message {
                Message::Request {
                    piece: actual_piece,
                    begin: actual_begin,
                    length,
                } => {
                    assert_eq!((actual_piece, actual_begin), (piece, begin));
                    assert_eq!(length, MB_BLOCK as u32);
                }
                other => panic!("expected v2 request, got {other:?}"),
            }
        }

        let request_bytes = outstanding
            .values()
            .map(|request| u64::from(request.length))
            .sum::<u64>();
        let owned_bytes = outstanding
            .values()
            .map(|request| request._memory_lease.as_ref().map_or(0, MemoryLease::bytes))
            .sum::<u64>();
        assert_eq!(owned_bytes, request_bytes);
        assert_eq!(
            resources.snapshot().classes[peer_buffer_index].used_bytes,
            baseline_used + request_bytes
        );

        for begin in [0, MB_BLOCK as u32] {
            let start = begin as usize;
            handle_v2_piece(
                &context,
                &state,
                &mut outstanding,
                &mut assemblies,
                &mut transfer_jobs,
                &mut pending_block_completions,
                0,
                begin,
                Bytes::copy_from_slice(&content[start..start + MB_BLOCK]),
            )
            .await
            .unwrap();
            start_ready_v2_piece_jobs(
                &context,
                &mut assemblies,
                &mut pending_piece_completions,
                &mut transfer_jobs,
            );
        }
        drain_v2_transfer_jobs(
            &context,
            &state,
            &mut assemblies,
            &mut pending_block_completions,
            &mut pending_piece_completions,
            &mut transfer_jobs,
        )
        .await
        .unwrap();
        let remaining_bytes = outstanding
            .values()
            .map(|request| u64::from(request.length))
            .sum::<u64>();
        assert!(!outstanding.contains_key(&(0, 0)));
        assert!(!outstanding.contains_key(&(0, MB_BLOCK as u32)));
        assert_eq!(remaining_bytes, (2 * MB_BLOCK) as u64);
        assert_eq!(
            resources.snapshot().classes[peer_buffer_index].used_bytes,
            baseline_used + remaining_bytes,
            "full piece storage and verification releases each response lease"
        );
        assert!(context.local_have.read().await.get(0));
        let payload_path = context.meta.files[0].path.resolve(&context.save_root);
        assert_eq!(
            &std::fs::read(payload_path).unwrap()[..MB_PIECE],
            &content[..MB_PIECE]
        );

        drop(outstanding); // peer disconnect releases the remaining requests
        assert_eq!(
            resources.snapshot().classes[peer_buffer_index].used_bytes,
            baseline_used
        );
    }

    #[tokio::test]
    async fn proof_layers_are_refused_for_unverified_content_then_cached() {
        let case = MultiBlock::new(4 * MB_PIECE, false).await;
        let file = case.task.meta.files[0].clone();
        let root = file.pieces_root.expect("layered v2 file has a root");
        let context = case
            .task
            .peer_context("127.0.0.1:6881".parse().unwrap(), PeerId::new());
        let leaves = (file.length / V2_BLOCK_SIZE as u64).max(1);

        // The file on disk is still the sentinel fill: it does not hash to the
        // metainfo root, so nothing may be served or cached.
        assert!(v2_proof_layers(&context, &file, root, leaves)
            .await
            .unwrap()
            .is_none());
        assert!(context.proof_cache.get(&root).is_none());

        // Real content proves against the root, is served, and is cached.
        let path = file.path.resolve(case.fx.temp.path());
        std::fs::write(&path, &case.content).unwrap();
        let first = v2_proof_layers(&context, &file, root, leaves)
            .await
            .unwrap()
            .expect("verified content yields layers");
        assert_eq!(first.last().unwrap()[0], root);
        let second = v2_proof_layers(&context, &file, root, leaves)
            .await
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second), "served from the cache");
    }

    #[test]
    fn proof_cache_is_byte_bounded_and_evicts_least_recently_used() {
        let cache = V2ProofCache::default();
        let layers = || Arc::new(vec![vec![[0u8; 32]]]);
        let third = V2_PROOF_CACHE_BYTES / 3 + 1;
        cache.insert([1; 32], layers(), third);
        cache.insert([2; 32], layers(), third);
        assert!(cache.get(&[1; 32]).is_some(), "touch 1 so 2 is the oldest");
        cache.insert([3; 32], layers(), third);
        assert!(cache.get(&[2; 32]).is_none(), "oldest entry evicted");
        assert!(cache.get(&[1; 32]).is_some() && cache.get(&[3; 32]).is_some());
        cache.insert([4; 32], layers(), V2_PROOF_CACHE_BYTES + 1);
        assert!(
            cache.get(&[4; 32]).is_none(),
            "an oversized entry is not kept"
        );
        assert!(cache.lock().bytes <= V2_PROOF_CACHE_BYTES);
    }
}
