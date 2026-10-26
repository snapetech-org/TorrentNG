#![allow(dead_code)] // Retained as the selectable v1/hybrid reference actor.

/// Per-torrent async task.
///
/// One tokio task per torrent owns: tracker announce loop, peer connection
/// management, piece picker, and storage writes.
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use rt_bencode::{BValue, Decoder};
use rusqlite::Connection;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, RwLock};
use tokio::time::{interval, sleep, timeout, Instant as TokioInstant, Sleep};
use tokio_util::codec::Framed;
use tracing::{debug, info, warn};
use url::Url;

use std::sync::Arc;

use rt_fastresume::{
    decide_recovery, should_audit_allocation, FastresumeState, FastresumeStore, FileHint,
    ImportPolicy, PartialPieceState, PieceState, RecheckReason, RecoveryAction, RecoveryInputs,
    MAX_FASTRESUME_BLOCKS_PER_PARTIAL_PIECE,
};
use rt_hash::V2_BLOCK_SIZE;
use rt_metainfo::{
    torrent_info_bytes_with_allocation_reservation, TorrentFileV1, TorrentMetaV1, TorrentMetaV2,
};
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};
use rt_path::{StorageProfile, StorageRootId};
use rt_peer_manager::{
    ChokeDecision, ChokeMode, ChokeState, Choker, PeerId, PeerSnapshot, DEFAULT_MAX_UNCHOKED,
};
use rt_peer_wire::{
    codec::PeerCodec,
    extension::{ExtensionHandshake, UtMetadataMessage, EXT_HANDSHAKE_ID},
    handshake::{ExtensionFlags, Handshake},
    message::Message,
};
use rt_piece_map::{FileSpan, PieceMap};
use rt_piece_picker::{Availability, BlockRequest, PieceAvailability, PiecePicker, MAX_BLOCK_SIZE};
use rt_session::{SessionRegistry, TorrentEntry, TorrentState};
use rt_storage::{
    scheduler::{scheduled_read_owned, scheduled_write, scheduled_write_with_shared_payload_lease},
    HybridV2PieceExpectation, IoClass, MountScheduler, PieceVerifier, SchedulerConfig,
    StorageIoConfig, VerifyResult,
};
use rt_tracker::{
    TrackerEvent, TrackerState, TrackerStatus, MAX_TRACKER_PEERS, MAX_TRACKER_STATE_ID_BYTES,
    MAX_TRACKER_STATE_TEXT_BYTES,
};
use rt_utp::UtpStream;

#[path = "torrent_task_reference/peer_connections.rs"]
mod peer_connections;
#[path = "torrent_task_reference/peer_session.rs"]
mod peer_session;
#[path = "torrent_task_reference/peer_transfer.rs"]
mod peer_transfer;

use crate::crash_safety::{self, CrashSafetyRuntime};
use crate::db_worker::DbExecutor;
use crate::egress_policy::{OutboundEgressPolicy, OutboundTargetKind};
use crate::network_budget::{
    GlobalNetworkBudget, HierarchicalRateLimiter, RateLimitCancellation, SharedRateLimiter,
};
use crate::torrent_task::{connect_utp_with_deadline, TorrentCmd};
use crate::tracker_runtime::{
    announce_tracker, bounded_response_body, first_tracker_key, next_tracker_key, protocol_numwant,
    tracker_keys_for_announce, tracker_keys_for_stopped_announce, url_log_target,
    TrackerAnnounceContext, TrackerAnnounceResult, TrackerAnnounceSpec, TrackerKey,
    TrackerScrapeResult, TrackerWorkerEvent, TrackerWorkers, MAX_TRACKER_ANNOUNCES_IN_FLIGHT,
    STOPPED_TRACKER_ANNOUNCE_DEADLINE,
};
use crate::transfer::coords::{BlockExtent, PieceGeometry};
use crate::transfer::obligations::{
    InboundDecision, InboundObligations, Reject as InboundReject,
    RejectReason as InboundRejectReason, Served as InboundServed,
};
use crate::{EnginePeerSnapshot, EngineTorrentLimits, EngineWebseedSnapshot, TorrentRuntimeStats};

const LOCAL_UT_METADATA_ID: u8 = 1;
const LOCAL_UT_PEX_ID: u8 = 2;
const METADATA_PIECE_SIZE: usize = 16 * 1024;
const PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

fn lock_prepared_files(files: &Mutex<HashSet<u32>>) -> MutexGuard<'_, HashSet<u32>> {
    match files.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Prepared-file membership is an optimization. Re-preparing is
            // safe: storage refuses to shrink existing payloads and length
            // setup is idempotent, so discard potentially stale membership.
            let mut guard = poisoned.into_inner();
            guard.clear();
            files.clear_poison();
            guard
        }
    }
}
const PEER_UPLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);
const PEER_SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
// Upload demand is bounded by the request queue and the pending-read cap
// below, with response bytes metered by the torrent and global upload
// buckets. There is deliberately no requests-per-interval ceiling: a count
// derived from a 64 MiB test fixture capped a continuously used connection at
// about 6.4 MiB/s and silently dropped valid requests beyond it.
const MAX_PENDING_UPLOAD_READS: usize = 16;
// A 64 MiB fixture at the protocol's 16 KiB block size is 4,096 requests.
// Requests are only small coordinates; the response data remains bounded by
// MAX_PENDING_UPLOAD_READS and the peer-buffer governor.
const MAX_QUEUED_UPLOAD_REQUESTS: usize = 4_096;
const MAX_UPLOAD_WORK: usize = MAX_PENDING_UPLOAD_READS + MAX_QUEUED_UPLOAD_REQUESTS;
const PEER_EVENT_SEND_TIMEOUT: Duration = Duration::from_millis(500);
// Terminal and seed-limit transitions must leave time for persistence and
// peer teardown. Tracker stop delivery is best effort and cannot consume the
// engine's entire command/shutdown budget when a tracker is unresponsive.
const STOPPED_TRACKER_CONTROL_ANNOUNCE_DEADLINE: Duration = Duration::from_secs(1);
const WEBSEED_RETRY_BASE: Duration = Duration::from_secs(1);
const WEBSEED_RETRY_MAX: Duration = Duration::from_secs(300);
const MAX_UT_PEX_PEERS: usize = 2_048;
// BEP 11 payloads arrive from untrusted peers inside a legal peer-wire
// message. Keep a flat bencode node bomb from reaching the parser's much
// larger generic default before the compact-peer count checks run.
const MAX_UT_PEX_BENCODE_NODES: usize = 16 * 1024;
// Peer snapshots are compatibility/API projections rather than a scheduling
// primitive. Keep a damaged or unusually configured torrent from allocating
// an unbounded response vector when an endpoint asks for its peers.
pub(crate) const MAX_PEER_SNAPSHOT_ITEMS: usize = 16_384;
const DOWNLOAD_BUCKET_MIN_CAPACITY: u64 = MAX_BLOCK_SIZE as u64;

fn peer_event_channel_capacity(max_peers: usize) -> usize {
    max_peers.clamp(64, 512)
}

fn download_bucket_capacity(limit: u64) -> u64 {
    limit.max(DOWNLOAD_BUCKET_MIN_CAPACITY)
}

fn consume_download_tokens(tokens: &mut u64, limit: u64, bytes: u64) -> bool {
    if *tokens < bytes {
        return false;
    }
    *tokens -= bytes;
    *tokens = (*tokens).min(download_bucket_capacity(limit));
    true
}

fn db_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn build_piece_map(piece_length: u64, files: &[TorrentFileV1]) -> Result<Arc<PieceMap>, String> {
    PieceMap::new_with_padding(
        piece_length,
        files
            .iter()
            .map(|file| FileSpan {
                file_index: file.index,
                path: file.path.clone(),
                content_offset: file.offset,
                length: file.length,
            })
            .collect(),
        files
            .iter()
            .filter_map(|file| file.pad.then_some(file.index)),
    )
    .map(Arc::new)
    .map_err(|error| format!("building torrent piece map: {error}"))
}

fn validate_padding_block(
    piece_map: &PieceMap,
    piece: u32,
    begin: u32,
    data: &[u8],
) -> anyhow::Result<()> {
    let length = u32::try_from(data.len())
        .map_err(|_| anyhow::anyhow!("peer block length does not fit in u32"))?;
    let regions = piece_map.validate_request(piece, begin, length)?;
    let mut data_offset = 0usize;
    for region in regions {
        let region_len = usize::try_from(region.length)
            .map_err(|_| anyhow::anyhow!("piece region length does not fit in memory"))?;
        let end = data_offset
            .checked_add(region_len)
            .ok_or_else(|| anyhow::anyhow!("piece region data offset overflow"))?;
        let region_data = data
            .get(data_offset..end)
            .ok_or_else(|| anyhow::anyhow!("peer block does not cover mapped piece regions"))?;
        if region.pad && region_data.iter().any(|byte| *byte != 0) {
            anyhow::bail!("peer block contains non-zero bytes for BEP 47 padding");
        }
        data_offset = end;
    }
    if data_offset != data.len() {
        anyhow::bail!("mapped piece regions do not cover the peer block");
    }
    Ok(())
}

const TORRENT_METADATA_MEMORY_BASE: usize = 64 * 1024;

fn add_owned_string_bytes(total: &mut usize, value: &String) {
    *total = total.saturating_add(value.capacity());
}

fn add_safe_path_bytes(total: &mut usize, path: &rt_path::SafeRelPath) {
    // SafeRelPath's component vector is private, but construction bounds its
    // capacity to the component count. Charge the vector and every owned
    // String rather than just the joined display length.
    *total = total.saturating_add(
        path.components()
            .len()
            .saturating_mul(std::mem::size_of::<String>()),
    );
    for component in path.components() {
        add_owned_string_bytes(total, component);
    }
}

fn add_v1_file_graph_bytes(
    total: &mut usize,
    files: &[TorrentFileV1],
    capacity: usize,
    entry_size: usize,
) {
    *total = total.saturating_add(capacity.saturating_mul(entry_size));
    for file in files {
        add_safe_path_bytes(total, &file.path);
    }
}

fn persistent_file_graph_memory_bytes(files: &[TorrentFileV1]) -> usize {
    let mut total = 0;
    add_v1_file_graph_bytes(
        &mut total,
        files,
        files.len(),
        std::mem::size_of::<TorrentFileV1>(),
    );
    add_v1_file_graph_bytes(
        &mut total,
        files,
        files.len(),
        std::mem::size_of::<FileSpan>(),
    );
    total
}

/// Estimate the allocations retained by one live v1 task outside the piece
/// hash/picker lease and the exact BEP9 upload-payload lease.
///
/// The task owns the parsed file graph, a second copy used to rebuild durable
/// file policy, and a third path graph inside PieceMap. It also materializes
/// tracker state and webseed bookkeeping from the parsed metadata. Admission
/// must reserve this lifetime memory before the task is constructed; otherwise
/// a session containing many large multi-file torrents can bypass the metadata
/// governor even though every individual parser input is bounded. Tracker
/// state is reserved separately after the durable tracker override is known.
pub(crate) fn persistent_torrent_metadata_memory_bytes(meta: &TorrentMetaV1) -> usize {
    let mut total = TORRENT_METADATA_MEMORY_BASE
        .saturating_add(std::mem::size_of::<TorrentMetaV1>())
        .saturating_add(std::mem::size_of::<PieceMap>())
        .saturating_add(2 * std::mem::size_of::<usize>());

    add_v1_file_graph_bytes(
        &mut total,
        &meta.files,
        meta.files.capacity(),
        std::mem::size_of::<TorrentFileV1>(),
    );
    // `metainfo_files` is a deep clone used to restore the original paths
    // after a durable file-policy projection changes them.
    total = total.saturating_add(persistent_file_graph_memory_bytes(&meta.files));

    add_owned_string_bytes(&mut total, &meta.name);
    if let Some(comment) = &meta.comment {
        add_owned_string_bytes(&mut total, comment);
    }
    if let Some(created_by) = &meta.created_by {
        add_owned_string_bytes(&mut total, created_by);
    }
    if let Some(announce) = &meta.announce {
        add_owned_string_bytes(&mut total, announce);
    }
    total = total.saturating_add(
        meta.announce_list
            .capacity()
            .saturating_mul(std::mem::size_of::<Vec<String>>()),
    );
    // tracker_tiers has one outer Vec and one inner Vec per non-empty source
    // tier, with a possible extra tier for the standalone announce URL.
    total = total.saturating_add(
        meta.announce_list
            .len()
            .saturating_add(1)
            .saturating_mul(std::mem::size_of::<Vec<TrackerState>>()),
    );
    for tier in &meta.announce_list {
        total = total.saturating_add(
            tier.capacity()
                .saturating_mul(std::mem::size_of::<String>()),
        );
        for tracker in tier {
            add_owned_string_bytes(&mut total, tracker);
        }
    }

    total = total.saturating_add(
        meta.webseeds
            .capacity()
            .saturating_mul(std::mem::size_of::<String>()),
    );
    for webseed in &meta.webseeds {
        add_owned_string_bytes(&mut total, webseed);
    }
    let webseed_count = meta.webseeds.len();
    total = total
        .saturating_add(webseed_count.saturating_mul(std::mem::size_of::<u8>()))
        .saturating_add(webseed_count.saturating_mul(std::mem::size_of::<Option<Instant>>()))
        .saturating_add(webseed_count.saturating_mul(std::mem::size_of::<i64>()))
        .saturating_add(webseed_count.saturating_mul(std::mem::size_of::<Option<Instant>>()));

    total
}

/// Estimate the v1 task graph plus the compact v2 verification projection
/// retained by a hybrid task. The original v2 paths, raw info dictionary,
/// tracker strings, and name are dropped after the projection is built.
pub(crate) fn persistent_hybrid_metadata_memory_bytes(
    v1: &TorrentMetaV1,
    v2: &TorrentMetaV2,
) -> usize {
    let payload_file_count = v1
        .files
        .iter()
        .filter(|file| !file.pad && file.length > 0)
        .count();
    let mut total = persistent_torrent_metadata_memory_bytes(v1)
        .saturating_add(std::mem::size_of::<HybridV2Meta>())
        .saturating_add(payload_file_count.saturating_mul(std::mem::size_of::<HybridV2File>()))
        .saturating_add(
            v2.piece_layers
                .capacity()
                .saturating_mul(std::mem::size_of::<([u8; 32], Vec<[u8; 32]>)>()),
        );
    for hashes in v2.piece_layers.values() {
        total = total.saturating_add(
            hashes
                .capacity()
                .saturating_mul(std::mem::size_of::<[u8; 32]>()),
        );
    }
    total
}

/// Estimate the complete retained tracker projection, including bounded
/// response fields that are not present in metainfo. The ID and status text
/// portions are reserved at their protocol bounds so a later announce cannot
/// grow a live task outside the governor after admission.
fn tracker_state_memory_bytes(tiers: &[Vec<TrackerState>], outer_capacity: usize) -> usize {
    let mut total = outer_capacity.saturating_mul(std::mem::size_of::<Vec<TrackerState>>());
    for tier in tiers {
        total = total.saturating_add(
            tier.capacity()
                .saturating_mul(std::mem::size_of::<TrackerState>()),
        );
        for tracker in tier {
            total = total
                .saturating_add(tracker.url.capacity())
                .saturating_add(MAX_TRACKER_STATE_ID_BYTES)
                .saturating_add(MAX_TRACKER_STATE_TEXT_BYTES);
        }
    }
    total
}

fn reserve_tracker_state_memory(
    resources: &ResourceGovernor,
    tiers: &[Vec<TrackerState>],
    outer_capacity: usize,
) -> Result<Vec<MemoryLease>, String> {
    let bytes = u64::try_from(tracker_state_memory_bytes(tiers, outer_capacity))
        .map_err(|_| "tracker state memory estimate does not fit in u64".to_owned())?;
    if bytes == 0 {
        return Ok(Vec::new());
    }
    resources
        .try_acquire(MemoryClass::Metadata, bytes)
        .map(|lease| vec![lease])
        .ok_or_else(|| format!("tracker state allocation of {bytes} bytes denied"))
}

fn parse_persisted_file_path(path: &str) -> Result<rt_path::SafeRelPath, String> {
    rt_path::SafeRelPath::from_slash_separated(path, cfg!(windows))
        .map_err(|error| format!("invalid persisted torrent file path {path:?}: {error}"))
}

fn restore_runtime_projection(entry: &mut TorrentEntry, previous: &TorrentEntry) {
    entry.total_length = previous.total_length;
    entry.amount_left = previous.amount_left;
    entry.finalizing = previous.finalizing;
    entry.state = previous.state;
    entry.completed_at = previous.completed_at;
    entry.error_message = previous.error_message.clone();
}

fn restore_progress_projection(entry: &mut TorrentEntry, previous: &TorrentEntry) {
    entry.total_length = previous.total_length;
    entry.amount_left = previous.amount_left;
    entry.finalizing = previous.finalizing;
}

/// Packed piece availability. A `Vec<bool>` uses one allocation and one
/// byte-ish slot per piece on the hot peer path; this representation keeps
/// the exact piece count while using one bit per piece. It is intentionally
/// private to the engine because wire/API compatibility still uses ordinary
/// bool vectors at the protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PieceBitmap {
    len: usize,
    words: Vec<u64>,
}

impl PieceBitmap {
    fn memory_bytes_for_len(len: usize) -> u64 {
        len.div_ceil(64).saturating_mul(std::mem::size_of::<u64>()) as u64
    }

    fn new(len: usize) -> Self {
        Self {
            len,
            words: vec![0; len.div_ceil(64)],
        }
    }

    /// A bitmap with every piece in `0..len` set. Used for BEP 6
    /// `HaveAll`, which stands in for a full `Bitfield` on the wire.
    fn all_true(len: usize) -> Self {
        Self {
            len,
            words: vec![u64::MAX; len.div_ceil(64)],
        }
    }

    #[cfg(test)]
    fn from_bools(bits: &[bool]) -> Self {
        let mut bitmap = Self::new(bits.len());
        for (index, value) in bits.iter().copied().enumerate() {
            if value {
                bitmap.set(index, true);
            }
        }
        bitmap
    }

    #[cfg(test)]
    fn from_bitfield(bits: &[u8], piece_count: usize) -> anyhow::Result<Self> {
        validate_bitfield_shape(bits, piece_count)?;
        Ok(Self::from_validated_bitfield(bits, piece_count))
    }

    fn from_validated_bitfield(bits: &[u8], piece_count: usize) -> Self {
        let mut bitmap = Self::new(piece_count);
        for (byte_index, byte) in bits.iter().copied().enumerate() {
            for bit in 0..8 {
                let piece = byte_index * 8 + bit;
                if piece >= piece_count {
                    break;
                }
                if byte & (0x80 >> bit) != 0 {
                    bitmap.set(piece, true);
                }
            }
        }
        bitmap
    }

    fn memory_bytes(&self) -> u64 {
        Self::memory_bytes_for_len(self.len)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, index: usize) -> Option<bool> {
        (index < self.len).then(|| self.words[index / 64] & (1_u64 << (index % 64)) != 0)
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
                let masked = if index + 1 == self.words.len() && !self.len.is_multiple_of(64) {
                    word & ((1_u64 << (self.len % 64)) - 1)
                } else {
                    *word
                };
                masked.count_ones() as usize
            })
            .sum()
    }

    fn first_set_u32(&self) -> Option<u32> {
        self.words.iter().enumerate().find_map(|(index, word)| {
            if *word == 0 {
                return None;
            }
            let piece = index
                .checked_mul(64)
                .and_then(|base| base.checked_add(word.trailing_zeros() as usize))?;
            u32::try_from(piece).ok()
        })
    }

    fn to_bitfield(&self) -> Vec<u8> {
        let mut bits = vec![0_u8; self.len.div_ceil(8)];
        for index in 0..self.len {
            if self.get(index).unwrap_or(false) {
                bits[index / 8] |= 0x80 >> (index % 8);
            }
        }
        bits
    }
}

impl PieceAvailability for PieceBitmap {
    fn has_piece(&self, piece: usize) -> bool {
        self.get(piece).unwrap_or(false)
    }
}

/// Messages from the engine to a running torrent task.
async fn receive_torrent_command(
    cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
    pending_command: &mut Option<TorrentCmd>,
) -> Option<TorrentCmd> {
    if pending_command.is_some() {
        pending_command.take()
    } else {
        cmd_rx.recv().await
    }
}

/// Run the best-effort stopped announce without stranding the actor. A
/// lifecycle command can arrive after its caller has already received a
/// durable acknowledgement; keep that command in the actor's normal FIFO
/// path while the network future is cancelled.
async fn await_stopped_announce_or_queue_command(
    task: &mut TorrentTask,
    cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
    pending_command: &mut Option<TorrentCmd>,
) {
    // Deciding which branch won, and restoring `task.stopped_announced`, must
    // happen after `operation` (which mutably borrows `task`) is dropped —
    // doing the restore assignment inside the `select!` arm itself conflicts
    // with that live borrow under NLL. So the arms only report an outcome;
    // `task` is touched once, below, outside the pinned future's scope.
    enum Outcome {
        Completed,
        Interrupted(Option<TorrentCmd>),
    }

    let outcome = {
        let operation = task.announce_stopped();
        tokio::pin!(operation);
        tokio::select! {
            _ = &mut operation => Outcome::Completed,
            command = cmd_rx.recv() => Outcome::Interrupted(command),
        }
    };

    if let Outcome::Interrupted(command) = outcome {
        // `announce_stopped` consumes the one-shot flag before it starts its
        // network work. If a later lifecycle command, or a closed command
        // channel, interrupts that future, no stopped request may have
        // reached any tracker. Restore the flag so the shutdown path can
        // make one bounded retry, and leave any received command pending in
        // the actor's normal FIFO path.
        task.stopped_announced = false;
        if let Some(command) = command {
            *pending_command = Some(command);
        }
    }
}

fn reject_torrent_command_during_shutdown(command: TorrentCmd) {
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
            let _ = reply.send(Default::default());
        }
        TorrentCmd::GetPeerSummary { reply } => {
            let _ = reply.send(crate::command::TorrentPeerStats::default());
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

fn reject_pending_torrent_commands(
    cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
    pending_command: &mut Option<TorrentCmd>,
) {
    if let Some(command) = pending_command.take() {
        reject_torrent_command_during_shutdown(command);
    }
    while let Ok(command) = cmd_rx.try_recv() {
        reject_torrent_command_during_shutdown(command);
    }
}

#[derive(Debug)]
enum RecheckOutcome {
    Complete,
    Paused {
        reply: Option<oneshot::Sender<Result<(), String>>>,
        previous_paused: bool,
        previous_restore_state: Option<TorrentState>,
        state_persisted: bool,
    },
    Cancelled,
    Shutdown,
    Failed(String),
}

struct RecheckInvalidPieces {
    values: Vec<i64>,
    total: usize,
}

impl RecheckInvalidPieces {
    fn new(piece_count: u32) -> Self {
        Self {
            values: Vec::with_capacity((piece_count as usize).min(rt_db::MAX_JOB_INVALID_PIECES)),
            total: 0,
        }
    }

    fn record(&mut self, piece: u32) {
        self.total = self.total.saturating_add(1);
        if self.values.len() < rt_db::MAX_JOB_INVALID_PIECES {
            self.values.push(piece as i64);
        }
    }
}

const JOB_STATE_RUNNING: &str = "running";
const JOB_STATE_PAUSED: &str = "paused";
const JOB_STATE_CANCELLING: &str = "cancelling";
const JOB_STATE_CANCELLED: &str = "cancelled";
const JOB_STATE_COMPLETED: &str = "completed";
const JOB_STATE_FAILED: &str = "failed";
const MAX_IN_MEMORY_PIECE_ASSEMBLIES: usize = 64;
const MAX_IN_MEMORY_PIECE_ASSEMBLY_BYTES_PER_TORRENT: usize = 64 * 1024 * 1024;
const PEER_REQUEST_PIPELINE_NORMAL: usize = 32;
const PEER_REQUEST_PIPELINE_CONSTRAINED: usize = 8;
/// Upper bound of the adaptive per-peer window (2 MiB of 16 KiB blocks).
const PEER_REQUEST_PIPELINE_ADAPTIVE_MAX: usize = 128;
/// Requests in flight across the whole torrent beyond which no peer's window
/// grows past the normal size (32 MiB of blocks).
const TORRENT_REQUESTS_ADAPTIVE_CAP: usize = 2_048;
/// Seconds of a peer's delivery rate the window should keep in flight.
const PEER_REQUEST_WINDOW_SECS: f64 = 0.25;
const TRACKER_PEER_CACHE_MIN: usize = 256;
const TRACKER_PEER_CACHE_MULTIPLIER: usize = 4;
// A HashSet bucket contains the socket address plus control bytes and can
// round capacity up during allocation. Reserve conservatively for the full
// bounded cache before accepting any tracker peers so later inserts cannot
// grow this state outside the governor.
const TRACKER_PEER_CACHE_BYTES_PER_ENTRY: u64 = 128;
// A dirty-piece watermark is only needed between storage-sync barriers. Keep
// the live set bounded; if more pieces become dirty than this, an unclean
// fastresume save deliberately carries no watermark and therefore falls back
// to a full recheck after a crash.
const MAX_RUNTIME_DIRTY_PIECES: usize = 65_536;

#[derive(Debug, Default)]
struct DirtyPieceTracker {
    pieces: HashSet<u32>,
    overflowed: bool,
}

impl DirtyPieceTracker {
    fn insert(&mut self, piece: u32) {
        if self.overflowed || self.pieces.contains(&piece) {
            return;
        }
        if self.pieces.len() >= MAX_RUNTIME_DIRTY_PIECES {
            self.pieces.clear();
            self.overflowed = true;
            return;
        }
        self.pieces.insert(piece);
    }

    fn len(&self) -> u64 {
        if self.overflowed {
            (MAX_RUNTIME_DIRTY_PIECES as u64).saturating_add(1)
        } else {
            self.pieces.len() as u64
        }
    }

    fn is_overflowed(&self) -> bool {
        self.overflowed
    }

    fn clear(&mut self) {
        self.pieces.clear();
        self.overflowed = false;
    }
}

/// Ceiling on pieces re-read by a `sample` completion verification, so a huge
/// torrent cannot turn a spot check into a near-full recheck.
const MAX_COMPLETION_SAMPLE_PIECES: usize = 16_384;
const COMPLETION_GATE_RETRY_MIN: Duration = Duration::from_secs(5);
const COMPLETION_GATE_RETRY_MAX: Duration = Duration::from_secs(60);
const MAX_REGRESSION_SAMPLE_PIECES: usize = 32;
const MAX_REGRESSION_FILES: usize = 20;

/// A finished download held back until its data is durable.
///
/// While it exists the torrent stays `Downloading` and reports one byte
/// remaining (see [`TorrentTask::reported_bytes_left`]), so no API consumer
/// sees a completed torrent whose data a power cut could still take back.
struct CompletionGate {
    runtime: Arc<CrashSafetyRuntime>,
    started: Instant,
    next_attempt: Instant,
    attempts: u32,
    settled: bool,
}

impl CompletionGate {
    fn new(runtime: Arc<CrashSafetyRuntime>) -> Self {
        runtime.record_gate_started();
        let now = Instant::now();
        CompletionGate {
            runtime,
            started: now,
            next_attempt: now,
            attempts: 0,
            settled: false,
        }
    }

    fn schedule_retry(&mut self) {
        let exponent = self.attempts.saturating_sub(1).min(4);
        let delay = COMPLETION_GATE_RETRY_MIN
            .saturating_mul(1u32 << exponent)
            .min(COMPLETION_GATE_RETRY_MAX);
        self.next_attempt = Instant::now() + delay;
    }

    fn release(mut self) {
        self.settled = true;
        self.runtime.record_gate_released();
    }
}

impl Drop for CompletionGate {
    fn drop(&mut self) {
        if !self.settled {
            self.runtime.record_gate_dropped();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarrierOutcome {
    /// Payload files were fsynced and the scheduler issued proof of the barrier.
    Synced,
    /// `fast` durability mode: no fsync by design.
    SkippedFast,
    /// The filesystem cannot fsync (e.g. some FUSE mounts); best effort.
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SaveOutcome {
    barrier: BarrierOutcome,
    persisted: bool,
}

impl SaveOutcome {
    /// Whether a completion may be released on the strength of this save.
    fn releases_completion(&self) -> bool {
        self.persisted
            && matches!(
                self.barrier,
                BarrierOutcome::Synced | BarrierOutcome::Unsupported
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyPlan {
    Sample,
    Full,
}

enum GateStep {
    /// Waiting for the barrier to succeed.
    Pending,
    /// The barrier succeeded; a read-back verification must run next.
    Verify(VerifyPlan),
    Released,
    /// The torrent is no longer complete; the gate was discarded.
    Dropped,
}

pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Pick the pieces a `sample` completion verification re-reads: the first and
/// last piece of every file (where a size-only preallocation most often leaves
/// an unwritten tail or head), plus a deterministic pseudo-random `percent` of
/// all pieces. Returns `None` when the sample would cover every piece anyway,
/// meaning a full verification is no more expensive.
fn select_completion_sample(
    piece_map: &PieceMap,
    files: &[(u32, u64, bool)],
    percent: u8,
    seed: u64,
) -> Option<Vec<u32>> {
    let count = piece_map.piece_count as usize;
    if count == 0 || percent >= 100 {
        return None;
    }
    let mut selected = std::collections::BTreeSet::new();
    for &(index, length, pad) in files {
        if pad || length == 0 {
            continue;
        }
        if let Some((first, _)) = piece_map.piece_range_for_file_bytes(index, 0, 1) {
            selected.insert(first);
        }
        if let Some((_, last)) = piece_map.piece_range_for_file_bytes(index, length - 1, length) {
            selected.insert(last.saturating_sub(1));
        }
    }
    let target = ((count as u64) * u64::from(percent)).div_ceil(100).max(1) as usize;
    let target = target
        .min(MAX_COMPLETION_SAMPLE_PIECES)
        .max(selected.len().min(MAX_COMPLETION_SAMPLE_PIECES));
    let mut rng = seed;
    let mut attempts = target.saturating_mul(4).saturating_add(16);
    while selected.len() < target && attempts > 0 {
        attempts -= 1;
        selected.insert((splitmix64(&mut rng) % count as u64) as u32);
    }
    if selected.len() >= count {
        return None;
    }
    let mut pieces: Vec<u32> = selected.into_iter().collect();
    pieces.truncate(MAX_COMPLETION_SAMPLE_PIECES.max(1));
    Some(pieces)
}

/// Blocking body of the startup allocation audit: which piece ranges overlap a
/// hole or unwritten extent of any file. Files that are missing or unreadable
/// are left to the file-hint check.
fn audit_files_blocking(
    piece_map: &PieceMap,
    files: Vec<(u32, PathBuf, u64)>,
) -> (Vec<(u32, u32)>, u64) {
    let mut ranges = Vec::new();
    let mut unsupported = 0u64;
    for (index, path, length) in files {
        match rt_storage::audit_file_allocation(&path, length) {
            Ok(rt_storage::AllocationAudit::Gaps { gaps, .. }) => {
                for (start, end) in gaps.merged() {
                    if let Some(range) = piece_map.piece_range_for_file_bytes(index, start, end) {
                        ranges.push(range);
                    }
                }
            }
            Ok(rt_storage::AllocationAudit::Unsupported) => unsupported += 1,
            Err(_) => {}
        }
    }
    (ranges, unsupported)
}

fn effective_piece_assembly_soft_cap(configured_bytes: usize) -> usize {
    configured_bytes.min(MAX_IN_MEMORY_PIECE_ASSEMBLY_BYTES_PER_TORRENT)
}

fn memory_aware_request_pipeline(piece_assembly_bytes: usize, soft_cap_bytes: usize) -> usize {
    if soft_cap_bytes == 0 {
        // A zero assembly cap disables aggregation, not downloading. Blocks
        // take the direct-write path in this mode, so keep the peer pipeline
        // open instead of permanently starving the torrent.
        return PEER_REQUEST_PIPELINE_NORMAL;
    }
    if piece_assembly_bytes.saturating_mul(4) >= soft_cap_bytes.saturating_mul(3) {
        PEER_REQUEST_PIPELINE_CONSTRAINED
    } else {
        PEER_REQUEST_PIPELINE_NORMAL
    }
}

/// Window for one peer: the fixed floor, grown toward `rate x delay / block`
/// only for a peer with proven sustained delivery, never under memory
/// pressure, and only while the whole torrent stays under its request cap. A
/// fixed 32-block window limited a peer to `512 KiB / response time` (about
/// 10 MiB/s at 50 ms) no matter how fast it could deliver.
fn adaptive_request_pipeline(
    memory_pipeline: usize,
    peer_rate_bytes_per_sec: f64,
    other_peers_outstanding: usize,
) -> usize {
    if memory_pipeline < PEER_REQUEST_PIPELINE_NORMAL
        || peer_rate_bytes_per_sec.is_nan()
        || peer_rate_bytes_per_sec <= 0.0
    {
        return memory_pipeline;
    }
    let wanted = (peer_rate_bytes_per_sec * PEER_REQUEST_WINDOW_SECS / f64::from(MAX_BLOCK_SIZE))
        .ceil() as usize;
    let headroom = TORRENT_REQUESTS_ADAPTIVE_CAP.saturating_sub(other_peers_outstanding);
    wanted
        .clamp(
            PEER_REQUEST_PIPELINE_NORMAL,
            PEER_REQUEST_PIPELINE_ADAPTIVE_MAX,
        )
        .min(headroom.max(PEER_REQUEST_PIPELINE_NORMAL))
}

#[cfg(test)]
fn stricter_limit(torrent_limit: Option<u64>, global_limit: Option<u64>) -> Option<u64> {
    match (torrent_limit, global_limit) {
        (Some(torrent), Some(global)) => Some(torrent.min(global)),
        (Some(limit), None) | (None, Some(limit)) => Some(limit),
        (None, None) => None,
    }
}

pub(crate) fn tracker_peer_cache_cap(max_peers: usize) -> usize {
    max_peers
        .saturating_mul(TRACKER_PEER_CACHE_MULTIPLIER)
        .clamp(TRACKER_PEER_CACHE_MIN, MAX_TRACKER_PEERS)
}

fn reserve_webseed_body_bytes(
    resources: &ResourceGovernor,
    bytes: u32,
) -> anyhow::Result<MemoryLease> {
    resources
        .try_acquire(MemoryClass::WebseedBody, u64::from(bytes))
        .ok_or_else(|| anyhow::anyhow!("webseed body allocation of {bytes} bytes denied"))
}

fn reserve_peer_upload_bytes(
    resources: &ResourceGovernor,
    bytes: u32,
) -> anyhow::Result<MemoryLease> {
    resources
        .try_acquire(MemoryClass::PeerBuffer, u64::from(bytes))
        .ok_or_else(|| anyhow::anyhow!("peer upload buffer allocation of {bytes} bytes denied"))
}

#[cfg(test)]
fn reserve_peer_bitfield_bytes(
    resources: &ResourceGovernor,
    pieces: &PieceBitmap,
) -> anyhow::Result<MemoryLease> {
    reserve_peer_bitfield_len(resources, pieces.len())
}

fn reserve_peer_bitfield_len(
    resources: &ResourceGovernor,
    piece_count: usize,
) -> anyhow::Result<MemoryLease> {
    let bytes = PieceBitmap::memory_bytes_for_len(piece_count);
    resources
        .try_acquire(MemoryClass::PeerBuffer, bytes)
        .ok_or_else(|| anyhow::anyhow!("peer bitfield allocation of {bytes} bytes denied"))
}

fn reserve_peer_bitfield_wire_bytes(
    resources: &ResourceGovernor,
    pieces: &PieceBitmap,
) -> anyhow::Result<MemoryLease> {
    let bitfield_bytes = u64::try_from(pieces.len().div_ceil(8))
        .map_err(|_| anyhow::anyhow!("peer bitfield wire allocation does not fit in u64"))?;
    // The payload is held by Message, Message::encode creates a second
    // length-prefixed copy, and TCP's framed writer can retain a third copy
    // until the send flushes. Reserve the peak rather than letting a large
    // torrent's piece count bypass the peer-buffer governor.
    let bytes = bitfield_bytes.saturating_mul(3).saturating_add(5);
    resources
        .try_acquire(MemoryClass::PeerBuffer, bytes)
        .ok_or_else(|| anyhow::anyhow!("peer bitfield wire allocation of {bytes} bytes denied"))
}

fn reserve_peer_event_bytes(
    resources: &ResourceGovernor,
    bytes: usize,
) -> anyhow::Result<MemoryLease> {
    let bytes = u64::try_from(bytes)
        .map_err(|_| anyhow::anyhow!("peer event allocation does not fit in u64"))?;
    resources
        .try_acquire(MemoryClass::PeerBuffer, bytes)
        .ok_or_else(|| anyhow::anyhow!("peer event allocation of {bytes} bytes denied"))
}

fn reserve_metadata_payload_bytes(
    resources: &ResourceGovernor,
    bytes: usize,
) -> anyhow::Result<MemoryLease> {
    let bytes = u64::try_from(bytes)
        .map_err(|_| anyhow::anyhow!("metadata payload allocation does not fit in u64"))?;
    resources
        .try_acquire(MemoryClass::Metadata, bytes)
        .ok_or_else(|| anyhow::anyhow!("metadata payload allocation of {bytes} bytes denied"))
}

fn prepare_metadata_payload(
    resources: &ResourceGovernor,
    info_hash: &str,
    raw: &[u8],
) -> (Option<Arc<Vec<u8>>>, Option<MemoryLease>) {
    let mut parse_memory_lease = match crate::engine::reserve_torrent_parse_memory(
        resources,
        raw.len(),
        "metadata upload extraction",
    ) {
        Ok(lease) => lease,
        Err(error) => {
            warn!(
                component = "memory",
                operation = "extract_upload_payload",
                torrent = %info_hash,
                result = "denied",
                error = %error,
                "disabling metadata upload because parser memory admission failed"
            );
            return (None, None);
        }
    };
    let mut reserve = |additional| {
        u64::try_from(additional)
            .map(|bytes| parse_memory_lease.try_grow(bytes))
            .unwrap_or(false)
    };
    match torrent_info_bytes_with_allocation_reservation(raw, &mut reserve) {
        Ok(metadata) => match reserve_metadata_payload_bytes(resources, metadata.len()) {
            Ok(lease) => (Some(Arc::new(metadata)), Some(lease)),
            Err(error) => {
                warn!(
                    component = "metadata",
                    operation = "reserve_upload_payload",
                    torrent = %info_hash,
                    result = "denied",
                    error = %error,
                    "disabling metadata upload because the metadata memory budget is exhausted"
                );
                (None, None)
            }
        },
        Err(error) => {
            warn!(
                component = "metadata",
                operation = "extract_upload_payload",
                torrent = %info_hash,
                result = "unavailable",
                error = %error,
                "disabling metadata upload because the torrent info dictionary is unavailable"
            );
            (None, None)
        }
    }
}

fn reserve_piece_assembly_bytes(
    resources: &ResourceGovernor,
    bytes: usize,
) -> anyhow::Result<MemoryLease> {
    let bytes = u64::try_from(bytes)
        .map_err(|_| anyhow::anyhow!("piece assembly allocation does not fit in u64"))?;
    resources
        .try_acquire(MemoryClass::PieceAssembly, bytes)
        .ok_or_else(|| anyhow::anyhow!("piece assembly allocation of {bytes} bytes denied"))
}

pub(crate) fn prepare_tracker_peer_cache(
    resources: &ResourceGovernor,
    max_peers: usize,
) -> (HashSet<SocketAddr>, Option<MemoryLease>) {
    let capacity = tracker_peer_cache_cap(max_peers);
    let bytes = u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(TRACKER_PEER_CACHE_BYTES_PER_ENTRY);
    let Some(memory_lease) = resources.try_acquire(MemoryClass::TrackerPeers, bytes) else {
        return (HashSet::new(), None);
    };

    let mut known = HashSet::new();
    if known.try_reserve(capacity).is_err() {
        // The governor lease is dropped with this return value, and the
        // cache remains disabled rather than allowing an infallible reserve
        // to turn allocator pressure into a process abort.
        return (HashSet::new(), None);
    }
    (known, Some(memory_lease))
}

#[cfg(test)]
fn remember_tracker_peers_bounded(
    known: &mut HashSet<SocketAddr>,
    peers: &[SocketAddr],
    cap: usize,
) -> u64 {
    let mut dropped = 0u64;
    for &peer in peers {
        if known.contains(&peer) {
            continue;
        }
        if known.len() >= cap {
            dropped = dropped.saturating_add(1);
            continue;
        }
        known.insert(peer);
    }
    dropped
}

/// A block received from a peer.
#[derive(Debug)]
pub struct BlockEvent {
    pub piece: u32,
    pub offset: u32,
    pub data: bytes::Bytes,
}

#[derive(Debug)]
struct LeasedBlockEvent {
    block: BlockEvent,
    _memory_lease: Option<Arc<MemoryLease>>,
}

const MAX_TRANSFER_COMPLETION_JOBS: usize = 64;

#[derive(Debug, Default)]
struct PendingPieceWork {
    generation: u64,
    writes_in_flight: usize,
    failed: bool,
    verify_ready: bool,
    verify_in_flight: bool,
    aggregate_write: bool,
    aggregate_write_ready: bool,
    verified_payload: Option<(bytes::Bytes, Option<Arc<MemoryLease>>)>,
}

enum TransferCompletion {
    BlockWritten {
        piece: u32,
        generation: u64,
        offset: u32,
        length: u64,
        result: Result<(), String>,
    },
    PieceVerified {
        piece: u32,
        generation: u64,
        result: VerifyResult,
        assembled: Option<(bytes::Bytes, Option<Arc<MemoryLease>>)>,
    },
    AggregatePieceWritten {
        piece: u32,
        generation: u64,
        result: Result<(), String>,
    },
}

type TransferJobFuture =
    Pin<Box<dyn std::future::Future<Output = TransferCompletion> + Send + 'static>>;

impl LeasedBlockEvent {
    fn new(block: BlockEvent, memory_lease: MemoryLease) -> anyhow::Result<Self> {
        let payload_bytes = u64::try_from(block.data.len())?;
        anyhow::ensure!(
            payload_bytes <= memory_lease.bytes(),
            "payload of {payload_bytes} bytes exceeds its {}-byte reservation",
            memory_lease.bytes()
        );
        Ok(Self {
            block,
            _memory_lease: Some(Arc::new(memory_lease)),
        })
    }

    fn with_shared_lease(block: BlockEvent, memory_lease: Option<Arc<MemoryLease>>) -> Self {
        Self {
            block,
            _memory_lease: memory_lease,
        }
    }

    fn with_optional_lease(
        block: BlockEvent,
        memory_lease: Option<MemoryLease>,
    ) -> anyhow::Result<Self> {
        match memory_lease {
            Some(memory_lease) => Self::new(block, memory_lease),
            None => Ok(Self::with_shared_lease(block, None)),
        }
    }

    #[cfg(test)]
    fn uncharged_for_test(piece: u32, offset: u32, data: bytes::Bytes) -> Self {
        Self {
            block: BlockEvent {
                piece,
                offset,
                data,
            },
            _memory_lease: None,
        }
    }

    fn len(&self) -> usize {
        self.block.data.len()
    }
}

#[derive(Debug)]
struct PieceAssembly {
    data: Vec<u8>,
    /// Once every block has arrived, freeze the original allocation into an
    /// immutable view. Hashing and file writes can then share it without
    /// copying the completed piece.
    completed_data: Option<bytes::Bytes>,
    received: Vec<bool>,
    last_used: Instant,
    /// The data allocation stays in the process-wide piece-assembly budget
    /// while this assembly or any shared payload view remains live. Tests
    /// that exercise the standalone value constructor may omit the lease;
    /// production insertion always uses `with_memory_lease`.
    _memory_lease: Option<Arc<MemoryLease>>,
}

#[derive(Debug)]
struct LeasedPiecePayload {
    data: Vec<u8>,
    /// The immutable Bytes owner holds this lease until its last shared view
    /// is dropped, even if the torrent task is cancelled during queued I/O.
    _memory_lease: Option<Arc<MemoryLease>>,
}

impl AsRef<[u8]> for LeasedPiecePayload {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl PieceAssembly {
    #[cfg(test)]
    fn new(len: usize) -> Self {
        Self::new_with_lease(len, None)
    }

    fn with_memory_lease(len: usize, memory_lease: MemoryLease) -> Self {
        Self::new_with_lease(len, Some(memory_lease))
    }

    fn new_with_lease(len: usize, memory_lease: Option<MemoryLease>) -> Self {
        Self {
            data: vec![0; len],
            completed_data: None,
            received: vec![false; len.div_ceil(MAX_BLOCK_SIZE as usize)],
            last_used: Instant::now(),
            _memory_lease: memory_lease.map(Arc::new),
        }
    }

    fn insert(&mut self, offset: u32, block: &[u8]) -> anyhow::Result<()> {
        self.last_used = Instant::now();
        let start = offset as usize;
        let end = start
            .checked_add(block.len())
            .ok_or_else(|| anyhow::anyhow!("piece block offset overflow"))?;
        if end > self.len() {
            anyhow::bail!(
                "piece block range {}..{} exceeds piece length {}",
                start,
                end,
                self.len()
            );
        }
        let block_idx = start / MAX_BLOCK_SIZE as usize;
        let Some(received) = self.received.get_mut(block_idx) else {
            anyhow::bail!("piece block index {block_idx} out of range");
        };
        if *received {
            if self.data()[start..end] == *block {
                return Ok(());
            }
            anyhow::bail!("conflicting duplicate block at offset {offset}");
        }
        self.data[start..end].copy_from_slice(block);
        *received = true;
        if self.is_complete() {
            self.completed_data = Some(bytes::Bytes::from_owner(LeasedPiecePayload {
                data: std::mem::take(&mut self.data),
                _memory_lease: self._memory_lease.clone(),
            }));
        }
        Ok(())
    }

    fn data(&self) -> &[u8] {
        self.completed_data
            .as_deref()
            .unwrap_or(self.data.as_slice())
    }

    fn is_complete(&self) -> bool {
        self.received.iter().all(|received| *received)
    }

    fn received_blocks(
        &self,
        resources: &ResourceGovernor,
        piece: u32,
    ) -> anyhow::Result<Vec<(u32, LeasedBlockEvent)>> {
        let mut blocks = Vec::new();
        for (block_idx, received) in self.received.iter().enumerate() {
            if !received {
                continue;
            }
            let start = block_idx.saturating_mul(MAX_BLOCK_SIZE as usize);
            let end = start
                .saturating_add(MAX_BLOCK_SIZE as usize)
                .min(self.len());
            if start >= end {
                continue;
            }
            let len = end - start;
            let offset = start as u32;
            if let Some(data) = &self.completed_data {
                blocks.push((
                    offset,
                    LeasedBlockEvent::with_shared_lease(
                        BlockEvent {
                            piece,
                            offset,
                            data: data.slice(start..end),
                        },
                        self._memory_lease.clone(),
                    ),
                ));
            } else {
                let lease = reserve_peer_event_bytes(resources, len)?;
                let data = bytes::Bytes::copy_from_slice(&self.data[start..end]);
                blocks.push((
                    offset,
                    LeasedBlockEvent::new(
                        BlockEvent {
                            piece,
                            offset,
                            data,
                        },
                        lease,
                    )?,
                ));
            }
        }
        Ok(blocks)
    }

    /// Split received ranges into immutable views that share the assembly's
    /// backing allocation. This is used when an assembly is evicted, so the
    /// spill path does not allocate another copy while memory is constrained.
    fn into_received_blocks(self, piece: u32) -> anyhow::Result<Vec<(u32, LeasedBlockEvent)>> {
        let Self {
            data,
            completed_data,
            received,
            _memory_lease,
            ..
        } = self;
        let data = completed_data.unwrap_or_else(|| bytes::Bytes::from(data));
        let memory_lease = _memory_lease;
        let mut blocks = Vec::new();
        blocks.try_reserve_exact(received.len())?;
        for (block_idx, is_received) in received.into_iter().enumerate() {
            if !is_received {
                continue;
            }
            let start = block_idx.saturating_mul(MAX_BLOCK_SIZE as usize);
            let end = start
                .saturating_add(MAX_BLOCK_SIZE as usize)
                .min(data.len());
            if start >= end {
                continue;
            }
            let offset = start as u32;
            blocks.push((
                offset,
                LeasedBlockEvent::with_shared_lease(
                    BlockEvent {
                        piece,
                        offset,
                        data: data.slice(start..end),
                    },
                    memory_lease.clone(),
                ),
            ));
        }
        Ok(blocks)
    }

    fn len(&self) -> usize {
        self.data().len()
    }
}

fn evict_piece_assemblies_to_budget(
    assemblies: &mut HashMap<u32, PieceAssembly>,
    assembly_bytes: &mut usize,
    current_piece: u32,
    max_assemblies: usize,
    max_bytes: usize,
) -> Vec<(u32, PieceAssembly)> {
    let mut evictions = Vec::new();
    while assemblies.len() > max_assemblies || *assembly_bytes > max_bytes {
        let Some(evict_piece) = assemblies
            .iter()
            .filter(|(piece, _)| **piece != current_piece)
            .min_by_key(|(_, assembly)| assembly.last_used)
            .map(|(piece, _)| *piece)
        else {
            break;
        };

        if let Some(assembly) = assemblies.remove(&evict_piece) {
            *assembly_bytes = assembly_bytes.saturating_sub(assembly.len());
            evictions.push((evict_piece, assembly));
        }
    }
    evictions
}

#[derive(Debug)]
enum PeerEvent {
    Bitfield {
        peer: SocketAddr,
        id: PeerId,
        pieces: PieceBitmap,
        _memory_lease: MemoryLease,
    },
    Have {
        peer: SocketAddr,
        id: PeerId,
        piece: u32,
    },
    Unchoked {
        peer: SocketAddr,
        id: PeerId,
        download_generation: u64,
    },
    Choked {
        peer: SocketAddr,
        id: PeerId,
        download_generation: u64,
        fast_negotiated: bool,
        /// Requests that reached the wire. Fast Choke keeps these alive;
        /// legacy Choke reports them for picker cancellation.
        outstanding: Vec<BlockRequest>,
    },
    Interested {
        peer: SocketAddr,
        id: PeerId,
    },
    NotInterested {
        peer: SocketAddr,
        id: PeerId,
    },
    Piece {
        peer: SocketAddr,
        id: PeerId,
        block: LeasedBlockEvent,
    },
    RequestRejected {
        peer: SocketAddr,
        id: PeerId,
        rejected: BlockRequest,
    },
    Uploaded {
        peer: SocketAddr,
        id: PeerId,
        bytes: u64,
    },
    Disconnected {
        peer: SocketAddr,
        id: PeerId,
        outstanding: Vec<BlockRequest>,
    },
    RequestTimedOut {
        peer: SocketAddr,
        id: PeerId,
        timed_out: Vec<BlockRequest>,
    },
    ExtendedHandshake {
        peer: SocketAddr,
        id: PeerId,
        ut_metadata_id: Option<u8>,
        ut_pex_id: Option<u8>,
        metadata_size: Option<u32>,
    },
    PeerExchange {
        peer: SocketAddr,
        id: PeerId,
        peers: Vec<SocketAddr>,
        dropped: Vec<SocketAddr>,
        _memory_lease: MemoryLease,
    },
}

impl PeerEvent {
    fn identity(&self) -> (SocketAddr, PeerId) {
        match self {
            Self::Bitfield { peer, id, .. }
            | Self::Have { peer, id, .. }
            | Self::Unchoked { peer, id, .. }
            | Self::Choked { peer, id, .. }
            | Self::Interested { peer, id }
            | Self::NotInterested { peer, id }
            | Self::Piece { peer, id, .. }
            | Self::RequestRejected { peer, id, .. }
            | Self::Uploaded { peer, id, .. }
            | Self::Disconnected { peer, id, .. }
            | Self::RequestTimedOut { peer, id, .. }
            | Self::ExtendedHandshake { peer, id, .. }
            | Self::PeerExchange { peer, id, .. } => (*peer, *id),
        }
    }
}

#[derive(Debug)]
struct PeerHandle {
    id: PeerId,
    cmd_tx: mpsc::Sender<PeerCommand>,
    upload_control: RateLimitCancellation,
    shutdown_control: RateLimitCancellation,
    /// Out-of-band cancellation for a peer whose command queue may be full.
    /// Control messages such as a ban must not depend on an untrusted peer
    /// draining a bounded command queue.
    abort: Option<tokio::task::AbortHandle>,
    peer_has: PieceBitmap,
    peer_have_count: usize,
    choked: bool,
    upload_choked: bool,
    interested: bool,
    downloaded: u64,
    uploaded: u64,
    download_rate: f64,
    upload_rate: f64,
    download_rate_window: u64,
    upload_rate_window: u64,
    download_rate_window_started: Instant,
    upload_rate_window_started: Instant,
    outstanding: usize,
    /// Invalidates requests still waiting in the peer command queue after a
    /// remote Choke. Requests already on the wire remain valid under Fast.
    download_generation: u64,
    /// Coordinates mirrored for picker cancellation; the peer loop owns each
    /// response-byte lease after its request command is accepted.
    requested: Vec<BlockRequest>,
    ut_metadata_id: Option<u8>,
    ut_pex_id: Option<u8>,
    metadata_size: Option<u32>,
    _peer_permit: OwnedSemaphorePermit,
    /// Dense per-peer availability maps are sized by the torrent's piece
    /// count. Keep their backing allocation under the process-wide peer
    /// memory budget for the full lifetime of the peer entry.
    _bitmap_memory_lease: MemoryLease,
    /// HAVE messages that could not enter the bounded peer mailbox. The
    /// bitmap keeps this retry state bounded by the torrent's piece map and
    /// avoids losing a protocol update when a peer is briefly backlogged.
    pending_have: PieceBitmap,
}

impl PeerHandle {
    fn replace_peer_have(&mut self, pieces: PieceBitmap) {
        self.peer_have_count = pieces.count_ones();
        self.peer_has = pieces;
    }

    fn add_peer_have(&mut self, piece: usize) -> bool {
        if self.peer_has.get(piece) != Some(false) {
            return false;
        }
        self.peer_has.set(piece, true);
        self.peer_have_count = self.peer_have_count.saturating_add(1);
        true
    }
}

#[derive(Debug)]
enum PeerCommand {
    Request {
        request: BlockRequest,
        /// A queued request from before the latest Choke is discarded after
        /// the peer loop advances its download generation.
        download_generation: u64,
        /// Acquired before enqueue; moves into the peer loop's outstanding
        /// request and then the matching received block event.
        memory_lease: MemoryLease,
    },
    /// Endgame loser cancellation. The peer loop keeps an on-wire request and
    /// its response lease outstanding because a Piece or Reject may still
    /// race the Cancel.
    Cancel {
        request: BlockRequest,
    },
    Have(u32),
    Choke,
    Unchoke,
    Shutdown,
}

struct UploadContext {
    save_root: PathBuf,
    // TNG-014: shared, not owned per peer -- PieceMap's `files: Vec<FileSpan>`
    // scales with file count, and a fresh deep clone on every new peer
    // connection (`upload_context()` below) was real, avoidable per-peer
    // memory and allocation cost for torrents with many files, at swarm
    // scale. `PieceMap` is only ever read after construction (never
    // mutated), so an `Arc` clone here is a cheap refcount bump instead.
    piece_map: Arc<PieceMap>,
    storage: MountScheduler,
    resources: ResourceGovernor,
    have_pieces: PieceBitmap,
    /// The upload-side availability bitmap is one more dense per-peer map.
    /// Keep its backing allocation charged until the peer task releases it.
    _bitmap_memory_lease: Option<MemoryLease>,
    metadata: Option<Arc<Vec<u8>>>,
    is_private: bool,
    pex_enabled: bool,
    /// One bucket per torrent, shared by every peer loop. A bucket per peer
    /// would let N peers each spend the full torrent allowance.
    torrent_upload: Arc<SharedRateLimiter>,
    /// BEP 6 was negotiated with this peer. A request we will not serve must
    /// then be answered with Reject Request instead of being dropped.
    fast_negotiated: bool,
    upload_control: RateLimitCancellation,
    shutdown_control: RateLimitCancellation,
    network_budget: GlobalNetworkBudget,
    global_download: Arc<HierarchicalRateLimiter>,
    global_upload: Arc<HierarchicalRateLimiter>,
}

#[derive(Clone)]
struct UploadReadContext {
    save_root: PathBuf,
    piece_map: Arc<PieceMap>,
    storage: MountScheduler,
    resources: ResourceGovernor,
    torrent_upload: Arc<SharedRateLimiter>,
    global_upload: Arc<HierarchicalRateLimiter>,
    upload_control: RateLimitCancellation,
}

struct LeasedUploadBlock {
    data: bytes::Bytes,
    _lease: MemoryLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UploadRequest {
    piece: u32,
    begin: u32,
    length: u32,
}

type UploadReadResult = (UploadRequest, anyhow::Result<LeasedUploadBlock>, bool);

/// Own the join handles for detached upload reads and abort them if the peer
/// loop is cancelled. Dropping a `JoinHandle` detaches its child task, so a
/// peer eviction or torrent shutdown could otherwise leave a read holding a
/// `MemoryLease` and scheduler work until its timeout.
#[derive(Default)]
struct UploadReadTasks(
    futures::stream::FuturesUnordered<tokio::task::JoinHandle<UploadReadResult>>,
);

impl std::ops::Deref for UploadReadTasks {
    type Target = futures::stream::FuturesUnordered<tokio::task::JoinHandle<UploadReadResult>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for UploadReadTasks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for UploadReadTasks {
    fn drop(&mut self) {
        abort_upload_reads(&self.0);
    }
}

#[derive(Debug, Clone)]
struct HybridV2Meta {
    piece_length: u64,
    files: Vec<HybridV2File>,
    piece_layers: HashMap<[u8; 32], Vec<[u8; 32]>>,
    valid_layout: bool,
}

#[derive(Debug, Clone)]
struct HybridV2File {
    /// The v1 file index used by `PieceMap`; v1 padding entries can make this
    /// differ from the file's v2 index.
    file_index: u32,
    length: u64,
    piece_offset: u64,
    pieces_root: Option<[u8; 32]>,
}

impl HybridV2Meta {
    fn from_validated_views(v1: &TorrentMetaV1, v2: TorrentMetaV2) -> Self {
        let v1_payload_count = v1
            .files
            .iter()
            .filter(|file| !file.pad && file.length > 0)
            .count();
        let mut files = Vec::with_capacity(v1_payload_count);
        let mut v1_files = v1.files.iter().filter(|file| !file.pad && file.length > 0);
        let mut valid_layout = v1.piece_length == v2.piece_length
            && v2.piece_length >= V2_BLOCK_SIZE as u64
            && v2.piece_length.is_power_of_two();

        for v2_file in v2.files.iter().filter(|file| !file.pad && file.length > 0) {
            let Some(v1_file) = v1_files.next() else {
                valid_layout = false;
                break;
            };
            if v1_file.length != v2_file.length || v1_file.offset != v2_file.piece_offset {
                valid_layout = false;
            }
            files.push(HybridV2File {
                file_index: v1_file.index,
                length: v2_file.length,
                piece_offset: v2_file.piece_offset,
                pieces_root: v2_file.pieces_root,
            });
        }
        if v1_files.next().is_some() {
            valid_layout = false;
        }

        Self {
            piece_length: v2.piece_length,
            files,
            piece_layers: v2.piece_layers,
            valid_layout,
        }
    }

    fn piece_expectation(&self, piece: u32) -> Option<HybridV2PieceExpectation> {
        if !self.valid_layout || self.piece_length == 0 {
            return None;
        }
        let piece_start = u64::from(piece).checked_mul(self.piece_length)?;
        let file_index = self
            .files
            .partition_point(|file| file.piece_offset <= piece_start)
            .checked_sub(1)?;
        let file = self.files.get(file_index)?;
        let file_end = file.piece_offset.checked_add(file.length)?;
        if piece_start >= file_end {
            return None;
        }

        let file_offset = piece_start.checked_sub(file.piece_offset)?;
        let content_len = file.length.checked_sub(file_offset)?.min(self.piece_length);
        let pieces_root = file.pieces_root?;
        let (expected_root, piece_leaf_count) = if file.length <= self.piece_length {
            (pieces_root, None)
        } else {
            let layer_index = usize::try_from(file_offset / self.piece_length).ok()?;
            let root = *self.piece_layers.get(&pieces_root)?.get(layer_index)?;
            let leaf_count = usize::try_from(self.piece_length / V2_BLOCK_SIZE as u64).ok()?;
            (root, Some(leaf_count))
        };

        Some(HybridV2PieceExpectation {
            file_index: file.file_index,
            file_offset,
            content_len,
            expected_root,
            piece_leaf_count,
        })
    }
}

#[derive(Clone)]
pub(super) struct TransferIoContext {
    meta: Arc<TorrentMetaV1>,
    piece_map: Arc<PieceMap>,
    save_root: PathBuf,
    storage: MountScheduler,
    prepared_files: Arc<Mutex<HashSet<u32>>>,
    prepared_file_locks: Arc<tokio::sync::Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>>,
}

async fn prepare_transfer_file_once(
    context: &TransferIoContext,
    file_index: u32,
    path: &std::path::Path,
    length: u64,
) -> anyhow::Result<()> {
    prepare_file_once_with_locks(
        &context.storage,
        &context.prepared_files,
        &context.prepared_file_locks,
        file_index,
        path,
        length,
    )
    .await
}

async fn prepare_file_once_with_locks(
    storage: &MountScheduler,
    prepared_files: &Mutex<HashSet<u32>>,
    prepared_file_locks: &tokio::sync::Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>,
    file_index: u32,
    path: &std::path::Path,
    length: u64,
) -> anyhow::Result<()> {
    let file_lock = {
        let mut locks = prepared_file_locks.lock().await;
        Arc::clone(
            locks
                .entry(file_index)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };
    let _file_guard = file_lock.lock().await;
    if lock_prepared_files(prepared_files).contains(&file_index) {
        return Ok(());
    }
    storage
        .prepare_file(path, length, storage.io_config().preallocation_mode)
        .await?;
    lock_prepared_files(prepared_files).insert(file_index);
    Ok(())
}

async fn write_transfer_block(
    context: &TransferIoContext,
    event: LeasedBlockEvent,
) -> anyhow::Result<()> {
    let LeasedBlockEvent {
        block,
        _memory_lease,
    } = event;
    let block_len = u32::try_from(block.data.len())
        .map_err(|_| anyhow::anyhow!("peer block length does not fit in u32"))?;
    context.piece_map.validate_request_block(BlockExtent::new(
        block.piece,
        block.offset,
        block_len,
    ))?;
    let regions = context
        .piece_map
        .validate_request(block.piece, block.offset, block_len)?;
    let mut data_offset = 0usize;

    for region in regions {
        let region_len = usize::try_from(region.length)
            .map_err(|_| anyhow::anyhow!("piece region length does not fit in memory"))?;
        let start = data_offset;
        let end = start
            .checked_add(region_len)
            .ok_or_else(|| anyhow::anyhow!("piece region data offset overflow"))?;
        let region_data = block
            .data
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("peer block does not cover mapped piece regions"))?;
        if region.pad {
            if region_data.iter().any(|byte| *byte != 0) {
                anyhow::bail!("peer block contains non-zero bytes for BEP 47 padding");
            }
            data_offset = end;
            continue;
        }
        let file = context
            .meta
            .files
            .iter()
            .find(|file| file.index == region.file_index)
            .ok_or_else(|| anyhow::anyhow!("file index {} out of range", region.file_index))?;
        let path = file.path.resolve(&context.save_root);
        prepare_transfer_file_once(context, file.index, &path, file.length).await?;
        data_offset = end;
        match _memory_lease.as_ref() {
            Some(lease) => {
                scheduled_write_with_shared_payload_lease(
                    &context.storage,
                    IoClass::PeerWrite,
                    &path,
                    region.file_offset,
                    block.data.slice(start..end),
                    true,
                    Arc::clone(lease),
                )
                .await?;
            }
            None => {
                scheduled_write(
                    &context.storage,
                    IoClass::PeerWrite,
                    &path,
                    region.file_offset,
                    bytes::Bytes::copy_from_slice(region_data),
                    true,
                )
                .await?;
            }
        }
    }

    if data_offset != block.data.len() {
        anyhow::bail!("mapped piece regions do not cover the peer block");
    }
    Ok(())
}

async fn write_transfer_piece(
    context: &TransferIoContext,
    piece: u32,
    data: bytes::Bytes,
    payload_lease: Option<Arc<MemoryLease>>,
) -> anyhow::Result<()> {
    let regions = context.piece_map.piece_to_file_regions(piece)?;
    for region in regions {
        if region.pad {
            continue;
        }
        let file = context
            .meta
            .files
            .iter()
            .find(|file| file.index == region.file_index)
            .ok_or_else(|| anyhow::anyhow!("file index {} out of range", region.file_index))?;
        let path = file.path.resolve(&context.save_root);
        prepare_transfer_file_once(context, file.index, &path, file.length).await?;
        let start = usize::try_from(region.piece_offset)
            .map_err(|_| anyhow::anyhow!("piece region offset does not fit memory"))?;
        let region_len = usize::try_from(region.length)
            .map_err(|_| anyhow::anyhow!("piece region length does not fit memory"))?;
        let end = start
            .checked_add(region_len)
            .ok_or_else(|| anyhow::anyhow!("piece region data offset overflow"))?;
        anyhow::ensure!(
            data.get(start..end).is_some(),
            "piece assembly does not cover mapped region"
        );
        let region_data = data.slice(start..end);
        match payload_lease.as_ref() {
            Some(lease) => {
                scheduled_write_with_shared_payload_lease(
                    &context.storage,
                    IoClass::PeerWrite,
                    &path,
                    region.file_offset,
                    region_data,
                    true,
                    Arc::clone(lease),
                )
                .await?;
            }
            None => {
                scheduled_write(
                    &context.storage,
                    IoClass::PeerWrite,
                    &path,
                    region.file_offset,
                    region_data,
                    true,
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn verify_transfer_piece(
    context: TransferIoContext,
    hybrid_v2: Option<Arc<HybridV2Meta>>,
    piece: u32,
    assembled: Option<(bytes::Bytes, Option<Arc<MemoryLease>>)>,
) -> VerifyResult {
    if let Some((data, payload_lease)) = assembled {
        let Some(expected) = context.meta.pieces.get(piece as usize) else {
            return VerifyResult::Missing {
                file_index: 0,
                reason: format!("no hash for piece {piece}"),
            };
        };
        if let Some(hybrid_v2) = hybrid_v2.as_ref() {
            let Some(expectation) = hybrid_v2.piece_expectation(piece) else {
                return VerifyResult::Missing {
                    file_index: 0,
                    reason: format!("no authenticated v2 file range for hybrid piece {piece}"),
                };
            };
            let Ok(piece_length) = usize::try_from(context.meta.piece_length) else {
                return VerifyResult::Missing {
                    file_index: expectation.file_index,
                    reason: "hybrid piece length does not fit memory address space".to_owned(),
                };
            };
            let Ok(v2_content_len) = usize::try_from(expectation.content_len) else {
                return VerifyResult::Missing {
                    file_index: expectation.file_index,
                    reason: "hybrid content length does not fit memory address space".to_owned(),
                };
            };
            let hashes = match payload_lease.as_ref() {
                Some(lease) => {
                    context
                        .storage
                        .hash_hybrid_piece_retry_queue_full_with_shared_payload_lease(
                            data,
                            v2_content_len,
                            piece_length,
                            Arc::clone(lease),
                        )
                        .await
                }
                None => {
                    context
                        .storage
                        .hash_hybrid_piece_retry_queue_full(data, v2_content_len, piece_length)
                        .await
                }
            };
            return match hashes {
                Ok(hashes) if hashes.v1_sha1 == *expected => {
                    let actual_v2 = match expectation.piece_leaf_count {
                        Some(_) => hashes.v2_piece_root,
                        None => hashes.v2_file_root,
                    };
                    if actual_v2 == expectation.expected_root {
                        VerifyResult::Valid
                    } else {
                        VerifyResult::Invalid
                    }
                }
                Ok(_) => VerifyResult::Invalid,
                Err(error) => VerifyResult::Missing {
                    file_index: expectation.file_index,
                    reason: error.to_string(),
                },
            };
        }
        let hash = match payload_lease.as_ref() {
            Some(lease) => {
                context
                    .storage
                    .hash_sha1_retry_queue_full_with_shared_payload_lease(data, Arc::clone(lease))
                    .await
            }
            None => context.storage.hash_sha1_retry_queue_full(data).await,
        };
        return match hash {
            Ok(actual) if &actual == expected => VerifyResult::Valid,
            Ok(_) => VerifyResult::Invalid,
            Err(error) => VerifyResult::Missing {
                file_index: 0,
                reason: error.to_string(),
            },
        };
    }

    let verifier = PieceVerifier::new(
        &context.save_root,
        &context.storage,
        &context.piece_map,
        &context.meta.pieces,
    );
    match hybrid_v2.as_ref() {
        Some(meta) => match meta.piece_expectation(piece) {
            Some(expectation) => verifier.verify_hybrid_piece(piece, &expectation).await,
            None => VerifyResult::Missing {
                file_index: 0,
                reason: format!("no authenticated v2 file range for hybrid piece {piece}"),
            },
        },
        None => verifier.verify_piece(piece).await,
    }
}

pub(crate) struct TorrentTask {
    info_hash_hex: String,
    meta: Arc<TorrentMetaV1>,
    hybrid_v2: Option<Arc<HybridV2Meta>>,
    /// Exact bencoded `info` bytes used for BEP 9 upload responses. This is
    /// shared by every peer instead of being rebuilt and copied per
    /// connection. The allocation is retained under the metadata budget for
    /// the lifetime of the torrent task.
    metadata: Option<Arc<Vec<u8>>>,
    _metadata_memory_lease: Option<MemoryLease>,
    /// Incremental reservation for a durable file-policy projection whose
    /// paths are larger than the original metainfo paths. The base metadata
    /// lease covers the original runtime graph; this lease tracks replacements
    /// applied by `apply_file_policy_from_db` and is refreshed on every path
    /// change so a database rename cannot bypass the governor.
    _file_policy_memory_lease: Option<MemoryLease>,
    /// Lifetime reservation for the parsed file/tracker graph retained by the
    /// task. This is separate from the exact BEP9 payload lease above because
    /// the latter can be disabled without discarding transfer metadata.
    _torrent_metadata_memory_lease: Option<MemoryLease>,
    /// Reservation for the current tracker tier projection. This is kept as
    /// one or more leases so an update can acquire only its growth before the
    /// old projection is replaced; a failed admission therefore leaves the
    /// previous tracker set and its accounting intact.
    _tracker_state_memory_leases: Vec<MemoryLease>,
    /// Backing storage for the picker and availability counts. The engine
    /// reserves this before constructing a live task so dense piece state
    /// cannot bypass the process-wide memory governor.
    _piece_index_memory_lease: Option<MemoryLease>,
    /// The immutable paths from the metainfo. `meta.files` is the runtime
    /// projection and may carry durable client-side rename overrides; keeping
    /// the wire paths lets a policy reload recover cleanly if the file
    /// projection is absent.
    metainfo_files: Vec<TorrentFileV1>,
    save_root: PathBuf,
    piece_map: Arc<PieceMap>,
    storage: MountScheduler,
    fastresume: FastresumeStore,
    tracker_tiers: Vec<Vec<TrackerState>>,
    active_tracker_tier: usize,
    private_tracker_key: Option<TrackerKey>,
    tracker_event: TrackerEvent,
    stopped_announced: bool,
    listen_port: u16,
    http_timeout: Duration,
    udp_timeout: Duration,
    min_announce_interval: Option<Duration>,
    registry: Arc<RwLock<SessionRegistry>>,
    db: DbExecutor,
    resources: ResourceGovernor,
    network_budget: GlobalNetworkBudget,
    cmd_rx: mpsc::Receiver<TorrentCmd>,
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_event_rx: mpsc::Receiver<PeerEvent>,
    peer_disconnect_tx: mpsc::Sender<PeerEvent>,
    peer_disconnect_rx: mpsc::Receiver<PeerEvent>,
    tracker_workers: TrackerWorkers,
    picker: PiecePicker,
    choker: Choker,
    /// active peer addresses
    active_peers: HashMap<SocketAddr, PeerHandle>,
    known_tracker_peers: HashSet<SocketAddr>,
    /// Explicit peers received while a startup/recovery recheck is draining
    /// the command queue. Admit them after the scan instead of dropping them.
    deferred_priority_peers: Vec<SocketAddr>,
    /// The cache is pre-sized and its reservation covers the hash table's
    /// bounded capacity for the task lifetime. This prevents tracker, DHT,
    /// PEX, and manual peer churn from growing an ungoverned address cache.
    _tracker_peer_cache_memory_lease: Option<MemoryLease>,
    last_peerless_reannounce: Option<Instant>,
    egress_policy: OutboundEgressPolicy,
    webseed_next_index: usize,
    webseed_failures: Vec<u8>,
    webseed_next_attempt: Vec<Option<Instant>>,
    webseed_last_rates: Vec<i64>,
    webseed_last_success: Vec<Option<Instant>>,
    last_progress_persist: Option<Instant>,
    transfer_stats_dirty: bool,
    piece_assemblies: HashMap<u32, PieceAssembly>,
    /// Partial pieces restored from fastresume already have their received
    /// bytes on disk, but their contents are not hydrated into an in-memory
    /// assembly. Keep those pieces on the direct-write path until they either
    /// verify or are rejected; otherwise a new block would be combined with
    /// zero-filled bytes and the completed piece would fail verification.
    restored_partial_pieces: HashSet<u32>,
    piece_assembly_bytes: usize,
    piece_assembly_soft_cap_bytes: usize,
    piece_assembly_evictions: u64,
    peer_request_window_reductions: u64,
    peer_command_queue_full: u64,
    peer_request_memory_retry_pending: bool,
    tracker_peer_cache_drops: u64,
    dirty_pieces_since_barrier: DirtyPieceTracker,
    /// Shared crash-safety settings, run verdict and counters. Defaults to an
    /// inert instance until the engine attaches the real one.
    crash_safety: Arc<CrashSafetyRuntime>,
    completion_gate: Option<CompletionGate>,
    /// A read-back verification queued by the completion gate, started by
    /// [`TorrentTask::drive_crash_safety`] where the command receiver is
    /// available.
    completion_verify_plan: Option<VerifyPlan>,
    /// The recheck now running (or about to run) is a completion verification.
    /// While set, the torrent keeps reporting one byte remaining.
    completion_verifying: bool,
    /// Pieces the next recheck should verify instead of the whole torrent.
    recheck_subset: Option<Vec<u32>>,
    /// A failed `sample` verification proved storage returned bad data; follow
    /// it with a full recheck.
    escalate_full_recheck: bool,
    /// The save path's filesystem cannot fsync; stop retrying.
    fsync_unsupported: bool,
    /// Why the startup recheck was forced by crash recovery, for event labels.
    recovery_recheck_reason: Option<RecheckReason>,
    /// Unix seconds of the last payload write. Persisted into fastresume to
    /// date recent writes for the post-host-crash recheck window.
    last_data_write_unix: std::sync::atomic::AtomicU64,
    super_seeding: bool,
    seed_ratio_limit: Option<f64>,
    seed_idle_limit: Option<Duration>,
    seeding_started_at: Option<Instant>,
    last_upload_at: Instant,
    download_limit_bytes_per_sec: Option<u64>,
    download_tokens: u64,
    download_tokens_updated: Instant,
    /// When the local download bucket will next hold enough tokens for a
    /// request that refilling had to skip. Nothing else restarts a peer whose
    /// outstanding requests drained while the bucket was empty.
    download_token_wake: Option<Instant>,
    torrent_upload: Arc<SharedRateLimiter>,
    torrent_download_limit_bytes_per_sec: Option<u64>,
    torrent_upload_limit_bytes_per_sec: Option<u64>,
    completed_piece_verify_from_memory: u64,
    completed_piece_verify_from_disk: u64,
    prepared_files: Arc<Mutex<HashSet<u32>>>,
    prepared_file_locks: Arc<tokio::sync::Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>>,
    transfer_jobs: futures::stream::FuturesUnordered<TransferJobFuture>,
    pending_transfer_blocks: HashMap<(u32, u32), u64>,
    pending_transfer_pieces: HashMap<u32, PendingPieceWork>,
    next_transfer_generation: u64,
    paused: bool,
    /// The lifecycle state to restore when a recheck was requested for a
    /// dormant/paused task. Dormant tasks are constructed with `paused = true`
    /// even when their durable state was active, so the runtime boolean alone
    /// cannot distinguish "paused by the user" from "temporarily quiesced
    /// until the recheck command arrives".
    recheck_restore_state: Option<TorrentState>,
    /// Durable state captured when a dormant task is promoted. Most dormant
    /// tasks stage through `Paused`, but an `Error` task must remain `Error`
    /// until an explicit resume/recheck command can move it to `Checking`.
    initial_state: TorrentState,
    max_peers: usize,
    torrent_max_peers: Option<usize>,
    pex_enabled: bool,
    event_retention: usize,
}

impl Drop for TorrentTask {
    fn drop(&mut self) {
        // The engine may have to abort this actor after a full mailbox or a
        // stuck storage/tracker operation. Dropping a Tokio AbortHandle does
        // not cancel the peer task it refers to, so explicitly abort every
        // peer here as the final lifecycle fallback. Otherwise handshakes
        // can keep sockets alive after the torrent actor has disappeared.
        for peer in self.active_peers.values() {
            if let Some(abort) = &peer.abort {
                abort.abort();
            }
        }
    }
}

impl TorrentTask {
    pub(crate) fn attach_torrent_metadata_memory_lease(&mut self, lease: MemoryLease) {
        self._torrent_metadata_memory_lease = Some(lease);
    }

    pub(crate) fn attach_hybrid_v2_metadata(&mut self, meta: TorrentMetaV2) {
        let projection = HybridV2Meta::from_validated_views(&self.meta, meta);
        if !projection.valid_layout {
            warn!(
                component = "torrent",
                operation = "hybrid_metadata_projection",
                torrent = %self.info_hash_hex,
                result = "invalid",
                "hybrid v2 metadata did not match the validated v1 file layout"
            );
        }
        self.hybrid_v2 = Some(Arc::new(projection));
    }

    /// Share the engine's crash-safety runtime with this task.
    pub(crate) fn attach_crash_safety(&mut self, runtime: Arc<CrashSafetyRuntime>) {
        self.crash_safety = runtime;
    }

    fn reserve_file_policy_memory(
        &self,
        effective_files: &[TorrentFileV1],
    ) -> Result<Option<MemoryLease>, String> {
        let baseline = persistent_file_graph_memory_bytes(&self.metainfo_files);
        let required = persistent_file_graph_memory_bytes(effective_files).saturating_sub(baseline);
        if required == 0 {
            return Ok(None);
        }
        let bytes = u64::try_from(required)
            .map_err(|_| "file-policy memory estimate does not fit in u64".to_owned())?;
        self.resources
            .try_acquire(MemoryClass::Metadata, bytes)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "file-policy allocation of {bytes} bytes denied for {} files",
                    effective_files.len()
                )
            })
    }

    fn reserve_file_policy_workspace_memory(&self) -> Result<Option<MemoryLease>, String> {
        let bytes = persistent_file_graph_memory_bytes(&self.metainfo_files);
        if bytes == 0 {
            return Ok(None);
        }
        let bytes = u64::try_from(bytes)
            .map_err(|_| "file-policy workspace memory estimate does not fit in u64".to_owned())?;
        self.resources
            .try_acquire(MemoryClass::Metadata, bytes)
            .map(Some)
            .ok_or_else(|| format!("file-policy workspace allocation of {bytes} bytes denied"))
    }

    fn refresh_tracker_state_memory(
        &mut self,
        tracker_tiers: &[Vec<TrackerState>],
        outer_capacity: usize,
    ) -> Result<(), String> {
        let required = u64::try_from(tracker_state_memory_bytes(tracker_tiers, outer_capacity))
            .map_err(|_| "tracker state memory estimate does not fit in u64".to_owned())?;
        let held = self
            ._tracker_state_memory_leases
            .iter()
            .map(MemoryLease::bytes)
            .fold(0u64, u64::saturating_add);

        if required > held {
            let extra = required - held;
            let lease = self
                .resources
                .try_acquire(MemoryClass::Metadata, extra)
                .ok_or_else(|| format!("tracker state allocation of {extra} bytes denied"))?;
            self._tracker_state_memory_leases.push(lease);
        } else if required == 0 {
            self._tracker_state_memory_leases.clear();
        } else if required < held {
            // Acquire before releasing the old reservation. If the temporary
            // double reservation does not fit, retaining the larger old lease
            // is safe and avoids an uncharged replacement window.
            if let Some(lease) = self.resources.try_acquire(MemoryClass::Metadata, required) {
                let old = std::mem::replace(&mut self._tracker_state_memory_leases, vec![lease]);
                drop(old);
            }
        }
        Ok(())
    }

    // This constructor is the current dependency-injection seam for a
    // torrent actor. It is intentionally explicit while the actor context is
    // being split into storage/network/persistence components.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn new(
        mut meta: TorrentMetaV1,
        save_root: PathBuf,
        paused: bool,
        initial_state: TorrentState,
        registry: Arc<RwLock<SessionRegistry>>,
        db: DbExecutor,
        resources: ResourceGovernor,
        cmd_rx: mpsc::Receiver<TorrentCmd>,
        fastresume_dir: PathBuf,
        max_peers: usize,
        listen_port: u16,
        http_timeout_secs: u64,
        udp_timeout_secs: u64,
        min_interval_secs: u64,
        piece_assembly_cap_bytes: usize,
        storage_io: StorageIoConfig,
        pex_enabled: bool,
        egress_policy: OutboundEgressPolicy,
        network_budget: GlobalNetworkBudget,
        event_retention: usize,
        piece_index_memory_lease: Option<MemoryLease>,
    ) -> Self {
        // Peer events are bounded per active torrent. Derive the capacity from
        // the daemon-wide peer ceiling so a deployment that deliberately runs
        // with a small peer budget does not retain 512 message slots per hot
        // torrent, while still keeping enough burst room for a normal
        // handshake/block event sequence. The upper bound prevents an
        // accidentally huge max_peers setting from multiplying memory.
        let peer_event_capacity = peer_event_channel_capacity(max_peers);
        let (peer_event_tx, peer_event_rx) = mpsc::channel(peer_event_capacity);
        let (peer_disconnect_tx, peer_disconnect_rx) = mpsc::channel(peer_event_capacity);
        let total = meta.total_length();
        let last_piece_len = if total.is_multiple_of(meta.piece_length) {
            meta.piece_length
        } else {
            total % meta.piece_length
        };
        let piece_count = meta.pieces.len();
        let webseed_failures = vec![0; meta.webseeds.len()];
        let webseed_next_attempt = vec![None; meta.webseeds.len()];
        let webseed_last_rates = vec![0; meta.webseeds.len()];
        let webseed_last_success = vec![None; meta.webseeds.len()];
        let picker = PiecePicker::new(piece_count, meta.piece_length as u32, last_piece_len as u32);
        let info_hash_hex: String = meta.info_hash.iter().map(|b| format!("{b:02x}")).collect();
        let metainfo_files = meta.files.clone();
        // The task only needs the exact info dictionary for BEP 9 uploads;
        // retaining the complete parsed .torrent blob here would keep up to
        // MAX_TORRENT_BYTES alive for every promoted torrent. Extract the
        // leased upload payload, then release the parser-owned raw blob.
        let raw = std::mem::take(&mut meta.raw);
        let (metadata, metadata_memory_lease) =
            prepare_metadata_payload(&resources, &info_hash_hex, &raw);
        drop(raw);
        let (known_tracker_peers, tracker_peer_cache_memory_lease) =
            prepare_tracker_peer_cache(&resources, max_peers);
        if tracker_peer_cache_memory_lease.is_none() {
            warn!(
                component = "memory",
                operation = "reserve_tracker_peer_cache",
                torrent = %info_hash_hex,
                result = "disabled",
                "tracker peer cache disabled because its memory budget or allocator reserve was unavailable"
            );
        }
        let piece_map = build_piece_map(meta.piece_length, &meta.files)
            .expect("metainfo parser rejects invalid piece maps");
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
        let metainfo_trackers = meta.all_trackers();
        let mut tracker_tiers = tracker_tiers_from_meta(&meta);
        let tracker_restore = db
            .run("restore_tracker_state", {
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
                // `torrents.trackers` is the durable client override. Keep
                // the metainfo's BEP-12 tiers for untouched torrents, but
                // treat a changed value (including an explicit empty list)
                // as authoritative after an actor restart.
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
                    operation = "restore_tracker_state",
                    torrent = %info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to restore durable tracker state; using fresh tracker session"
                );
            }
        }
        let tracker_state_memory_leases = match reserve_tracker_state_memory(
            &resources,
            &tracker_tiers,
            tracker_tiers.capacity(),
        ) {
            Ok(leases) => leases,
            Err(error) => {
                warn!(
                    component = "memory",
                    operation = "reserve_tracker_state",
                    torrent = %info_hash_hex,
                    result = "disabled",
                    error = %error,
                    "tracker state disabled because its memory budget or allocator reserve was unavailable"
                );
                // Assign a fresh empty vector rather than clearing the
                // current one: Vec::clear retains its outer allocation.
                tracker_tiers = Vec::new();
                Vec::new()
            }
        };
        let private_tracker_key = if meta.private {
            first_tracker_key(&tracker_tiers)
        } else {
            None
        };
        let mut task = TorrentTask {
            info_hash_hex,
            meta: Arc::new(meta),
            hybrid_v2: None,
            metadata,
            _metadata_memory_lease: metadata_memory_lease,
            _file_policy_memory_lease: None,
            _torrent_metadata_memory_lease: None,
            _tracker_state_memory_leases: tracker_state_memory_leases,
            _piece_index_memory_lease: piece_index_memory_lease,
            metainfo_files,
            save_root,
            piece_map,
            storage,
            fastresume: FastresumeStore::new(fastresume_dir),
            tracker_tiers,
            active_tracker_tier: 0,
            private_tracker_key,
            tracker_event: TrackerEvent::Started,
            stopped_announced: paused,
            listen_port,
            http_timeout: Duration::from_secs(http_timeout_secs.max(1)),
            udp_timeout: Duration::from_secs(udp_timeout_secs.max(1)),
            min_announce_interval: (min_interval_secs > 0)
                .then(|| Duration::from_secs(min_interval_secs)),
            registry,
            db,
            resources,
            network_budget,
            cmd_rx,
            peer_event_tx,
            peer_event_rx,
            peer_disconnect_tx,
            peer_disconnect_rx,
            tracker_workers: TrackerWorkers::new(),
            picker,
            choker: Choker::new(DEFAULT_MAX_UNCHOKED),
            active_peers: HashMap::new(),
            known_tracker_peers,
            deferred_priority_peers: Vec::new(),
            _tracker_peer_cache_memory_lease: tracker_peer_cache_memory_lease,
            last_peerless_reannounce: None,
            egress_policy,
            webseed_next_index: 0,
            webseed_failures,
            webseed_next_attempt,
            webseed_last_rates,
            webseed_last_success,
            last_progress_persist: None,
            transfer_stats_dirty: false,
            piece_assemblies: HashMap::new(),
            restored_partial_pieces: HashSet::new(),
            piece_assembly_bytes: 0,
            piece_assembly_soft_cap_bytes: effective_piece_assembly_soft_cap(
                piece_assembly_cap_bytes,
            ),
            piece_assembly_evictions: 0,
            peer_request_window_reductions: 0,
            peer_command_queue_full: 0,
            peer_request_memory_retry_pending: false,
            tracker_peer_cache_drops: 0,
            dirty_pieces_since_barrier: DirtyPieceTracker::default(),
            crash_safety: CrashSafetyRuntime::inert(),
            completion_gate: None,
            completion_verify_plan: None,
            completion_verifying: false,
            recheck_subset: None,
            escalate_full_recheck: false,
            fsync_unsupported: false,
            recovery_recheck_reason: None,
            last_data_write_unix: std::sync::atomic::AtomicU64::new(0),
            super_seeding: false,
            seed_ratio_limit: None,
            seed_idle_limit: None,
            seeding_started_at: None,
            last_upload_at: Instant::now(),
            download_limit_bytes_per_sec: None,
            download_tokens: u64::MAX,
            download_tokens_updated: Instant::now(),
            download_token_wake: None,
            torrent_upload: Arc::new(SharedRateLimiter::new(None)),
            torrent_download_limit_bytes_per_sec: None,
            torrent_upload_limit_bytes_per_sec: None,
            completed_piece_verify_from_memory: 0,
            completed_piece_verify_from_disk: 0,
            prepared_files: Arc::new(Mutex::new(HashSet::new())),
            prepared_file_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            transfer_jobs: futures::stream::FuturesUnordered::new(),
            pending_transfer_blocks: HashMap::new(),
            pending_transfer_pieces: HashMap::new(),
            next_transfer_generation: 0,
            paused,
            recheck_restore_state: match initial_state {
                TorrentState::Paused => Some(TorrentState::Paused),
                TorrentState::Stopped => Some(TorrentState::Stopped),
                _ => None,
            },
            initial_state,
            max_peers,
            torrent_max_peers: None,
            pex_enabled,
            event_retention,
        };
        if let Err(error) = task.apply_torrent_limits_from_db().await {
            // Starting with an unbounded picker after a persisted-policy
            // read failed is unsafe: a damaged or unavailable database must
            // not silently widen the set of pieces we may write.
            task.fail_closed_runtime_policy("restore_runtime_policy", &error);
        }
        task
    }

    pub async fn run(mut self) {
        // Keep the engine command receiver outside the actor while it runs so
        // a webseed fetch can be selected against lifecycle commands without
        // borrowing two fields of `self` through overlapping futures. The
        // replacement is never observed: this task owns the real receiver
        // until it exits.
        let (_, replacement_cmd_rx) = mpsc::channel(1);
        let mut cmd_rx = std::mem::replace(&mut self.cmd_rx, replacement_cmd_rx);
        let mut pending_command = None;
        let restored = self.restore_fastresume().await;
        self.persist_tracker_state().await;
        if self.paused {
            self.persist_progress().await;
            // Dormant promotion constructs the task paused so no transfer
            // starts before the queued command is delivered. Preserve an
            // explicitly stopped projection during that staging window;
            // rewriting it as Paused makes a stopped torrent resume with the
            // wrong lifecycle state if the process exits before the command.
            let startup_state = match self.initial_state {
                TorrentState::Error => TorrentState::Error,
                // Promotion stages every dormant task paused. Preserve a
                // queued lifecycle projection during that staging window;
                // otherwise a process exit before the queued command arrives
                // silently turns queue admission into a user pause.
                TorrentState::Queued => TorrentState::Queued,
                _ => self.recheck_restore_state.unwrap_or(TorrentState::Paused),
            };
            if let Err(error) = self.set_state_checked(startup_state).await {
                warn!(
                    component = "torrent",
                    operation = "startup_state",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to persist paused startup state; stopping task"
                );
                reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
                return;
            }
        } else if self.initial_state == TorrentState::Checking || !restored {
            // `Checking` is a durable promise that the previous process had
            // not completed verification. A fastresume file may still exist
            // from before that scan, so it must never be allowed to turn the
            // task directly into Downloading/Seeding on restore. The engine
            // also recovers a queued recheck job after tasks are loaded; the
            // scan below can adopt that job id when its command arrives.
            match self
                .run_recheck_with_receiver(&mut cmd_rx, &mut pending_command, None)
                .await
            {
                RecheckOutcome::Shutdown => {
                    reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
                    return;
                }
                RecheckOutcome::Failed(error) => {
                    warn!(
                        component = "torrent",
                        operation = "startup_recheck",
                        torrent = %self.info_hash_hex,
                        result = "error",
                        error = %error,
                        "startup recheck failed; stopping task"
                    );
                    reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
                    return;
                }
                RecheckOutcome::Complete
                | RecheckOutcome::Paused { .. }
                | RecheckOutcome::Cancelled => {}
            }
        } else if self.picker.is_complete() {
            if let Err(error) = self.set_state_checked(TorrentState::Seeding).await {
                warn!(
                    component = "torrent",
                    operation = "startup_state",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to persist seeding startup state; stopping task"
                );
                reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
                return;
            }
        } else if let Err(error) = self.set_state_checked(TorrentState::Downloading).await {
            warn!(
                component = "torrent",
                operation = "startup_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist downloading startup state; stopping task"
            );
            reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
            return;
        }

        let mut choke_tick = interval(Duration::from_secs(10));
        let mut tracker_tick = interval(Duration::from_secs(5));
        let mut peer_retry_tick = interval(Duration::from_secs(30));
        let mut peer_control_retry_tick = interval(Duration::from_secs(1));
        // Webseeds are a fallback path, not the primary piece scheduler. A
        // fixed interval would still wake every task even while it is paused,
        // complete, peer-connected, or webseed-free. A deadline-driven sleep
        // wakes only when the guarded work can run; after a failure it follows
        // that seed's exponential retry deadline.
        let mut webseed_sleep = Box::pin(sleep(WEBSEED_RETRY_MAX));
        reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());

        loop {
            tokio::select! {
                command = receive_torrent_command(&mut cmd_rx, &mut pending_command) => {
                    let Some(cmd) = command else {
                        // The owning engine is gone. Persist the last safe
                        // state and close peer tasks instead of leaving a
                        // detached torrent actor running on timers forever.
                        warn!(
                            component = "torrent",
                            operation = "run",
                            torrent = %self.info_hash_hex,
                            result = "command_channel_closed",
                            "torrent command channel closed; shutting down"
                        );
                        self.cancel_tracker_announces();
                        self.announce_stopped_with_control_deadline().await;
                        self.drain_transfer_jobs().await;
                        self.persist_progress().await;
                        self.save_fastresume_for_exit().await;
                        self.shutdown_peers().await;
                        break;
                    };
                    match cmd {
                        TorrentCmd::Shutdown => {
                            self.cancel_tracker_announces();
                            self.announce_stopped_with_control_deadline().await;
                            self.drain_transfer_jobs().await;
                            self.persist_progress().await;
                            self.save_fastresume_for_exit().await;
                            self.shutdown_peers().await;
                            break;
                        }
                        TorrentCmd::Pause { reply } => {
                            let was_paused = self.paused;
                            self.paused = true;
                            self.cancel_tracker_announces();
                            self.drain_transfer_jobs().await;
                            self.save_fastresume(false).await;
                            self.shutdown_peers().await;
                            let result = self.set_state_checked(TorrentState::Paused).await;
                            if let Err(error) = &result {
                                // A failed state write must not turn a
                                // rejected pause into a runtime-only pause.
                                // Restore the prior control state and let the
                                // caller retry once the durable store is
                                // available again.
                                self.paused = was_paused;
                                if !was_paused {
                                    self.restart_tracker_session();
                                }
                                warn!(
                                    component = "torrent",
                                    operation = "pause",
                                    torrent = %self.info_hash_hex,
                                    result = "error",
                                    error = %error,
                                    "failed to persist torrent pause"
                                );
                            } else {
                                self.recheck_restore_state = Some(TorrentState::Paused);
                                self.tracker_event = TrackerEvent::Started;
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(result);
                            }
                            if self.paused {
                                // Tracker shutdown is best-effort lifecycle
                                // housekeeping. Do it after the durable pause
                                // acknowledgement so a slow tracker cannot
                                // make the caller time out with a task that is
                                // already safely paused.
                                await_stopped_announce_or_queue_command(
                                    &mut self,
                                    &mut cmd_rx,
                                    &mut pending_command,
                                )
                                .await;
                            }
                        }
                        TorrentCmd::Resume { reply } => {
                            let result = self.prepare_resume().await;
                            if let Some(reply) = reply {
                                // Do not make API callers wait for a full
                                // disk recheck. `Checking` is the durable
                                // acknowledgement; the recheck continues in
                                // this actor immediately afterward.
                                let _ = reply.send(result.clone());
                            }
                            if result.is_ok() {
                                match self
                                    .run_recheck_with_receiver(
                                        &mut cmd_rx,
                                        &mut pending_command,
                                        None,
                                    )
                                    .await
                                {
                                    RecheckOutcome::Shutdown => break,
                                    RecheckOutcome::Failed(error) => {
                                        warn!(
                                            component = "torrent",
                                            operation = "resume_recheck",
                                            torrent = %self.info_hash_hex,
                                            result = "error",
                                            error = %error,
                                            "resume recheck failed; stopping task"
                                        );
                                        break;
                                    }
                                    RecheckOutcome::Complete
                                    | RecheckOutcome::Paused { .. }
                                    | RecheckOutcome::Cancelled => {}
                                }
                            }
                            // A recheck can invalidate pieces after the
                            // webseed timer has backed off because the picker
                            // was complete. Wake the scheduler immediately so
                            // repair does not wait for the long retry deadline.
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                        TorrentCmd::QuiesceForStorageMove { reply } => {
                            let was_paused = self.paused;
                            self.paused = true;
                            self.cancel_tracker_announces();
                            self.drain_transfer_jobs().await;
                            self.save_fastresume(false).await;
                            self.shutdown_peers().await;
                            if let Err(error) = self.set_state_checked(TorrentState::Paused).await {
                                self.paused = was_paused;
                                if !was_paused {
                                    self.restart_tracker_session();
                                }
                                let _ = reply.send(Err(format!(
                                    "failed to persist torrent quiesce state: {error}"
                                )));
                                continue;
                            }
                            self.tracker_event = TrackerEvent::Started;
                            // `shutdown_peers` above terminates every peer
                            // task, so no *new* PeerEvent can arrive after
                            // it returns -- but an event already buffered
                            // in the channel a moment before disconnect
                            // could still be sitting there. Drop anything
                            // left so a leftover Block event can't reach
                            // `handle_block` (and write to disk) after we
                            // hand back this reply.
                            while self.peer_event_rx.try_recv().is_ok() {}
                            let _ = reply.send(Ok(was_paused));
                            await_stopped_announce_or_queue_command(
                                &mut self,
                                &mut cmd_rx,
                                &mut pending_command,
                            )
                            .await;
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
                                // Any file-prepared bookkeeping refers to
                                // handles/allocations at the old path; the
                                // files now live at a verified-identical
                                // new path, so start clean rather than
                                // trust stale state across the move.
                                lock_prepared_files(&self.prepared_files).clear();
                            }
                            if !resume_paused {
                                match self.prepare_resume().await {
                                    Ok(()) => {
                                        // `Checking` is the durable handoff
                                        // point. Do not make storage-job
                                        // completion wait for a full disk
                                        // recheck, but do report failures that
                                        // would otherwise leave the task
                                        // paused after the job was marked
                                        // complete.
                                        let _ = reply.send(Ok(()));
                                        match self
                                            .run_recheck_with_receiver(
                                                &mut cmd_rx,
                                                &mut pending_command,
                                                None,
                                            )
                                            .await
                                        {
                                            RecheckOutcome::Shutdown => break,
                                            RecheckOutcome::Failed(error) => {
                                                warn!(
                                                    component = "torrent",
                                                    operation = "storage_move_resume_recheck",
                                                    torrent = %self.info_hash_hex,
                                                    result = "error",
                                                    error = %error,
                                                    "post-storage-move recheck failed; stopping task"
                                                );
                                                break;
                                            }
                                            RecheckOutcome::Complete
                                            | RecheckOutcome::Paused { .. }
                                            | RecheckOutcome::Cancelled => {}
                                        }
                                    }
                                    Err(error) => {
                                        let _ = reply.send(Err(error.clone()));
                                        warn!(
                                            component = "torrent",
                                            operation = "storage_move_resume",
                                            torrent = %self.info_hash_hex,
                                            result = "error",
                                            error = %error,
                                            "post-storage-move resume could not be persisted"
                                        );
                                    }
                                }
                                reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                            } else {
                                let _ = reply.send(Ok(()));
                            }
                        }
                        TorrentCmd::NewPeers(addrs) => {
                            if !self.paused {
                                if !self.meta.private {
                                    self.remember_tracker_peers(&addrs);
                                }
                                self.connect_peers(addrs, PeerSource::Dht).await;
                            }
                        }
                        TorrentCmd::PriorityPeers(addrs) => {
                            if !self.paused {
                                // Private torrents have no tracker/DHT/PEX
                                // peer source. An explicit peer supplied by a
                                // client is therefore the authority that
                                // admits this address for the connection and
                                // for later reconnects.
                                if self.meta.private {
                                    self.remember_tracker_peers(&addrs);
                                }
                                self.connect_priority_peers(addrs).await;
                            }
                        }
                        TorrentCmd::BanPeer(peer) => {
                            self.evict_peer(peer);
                        }
                        TorrentCmd::EvictBannedPeers => {
                            self.evict_banned_peers().await;
                        }
                        TorrentCmd::GetPeers { reply } => {
                            let _ = reply.send(Vec::new());
                        }
                        TorrentCmd::GetPeerSnapshotCount { reply } => {
                            let _ = reply.send(self.active_peers.len());
                        }
                        TorrentCmd::GetPeerSnapshots {
                            max_entries,
                            reply,
                        } => {
                            let _ = reply.send(self.peer_snapshots_limited(max_entries));
                        }
                        TorrentCmd::GetWebseeds { reply } => {
                            let _ = reply.send(self.webseed_snapshots());
                        }
                        TorrentCmd::GetRuntimeStats { reply } => {
                            let _ = reply.send(self.runtime_stats());
                        }
                        TorrentCmd::GetPeerSummary { reply } => {
                            let _ = reply.send(self.peer_stats());
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
                        }
                        TorrentCmd::Recheck { job_id } => {
                            match self
                                .run_recheck_with_receiver(
                                    &mut cmd_rx,
                                    &mut pending_command,
                                    job_id,
                                )
                                .await
                            {
                                RecheckOutcome::Shutdown => break,
                                RecheckOutcome::Failed(error) => {
                                    warn!(
                                        component = "torrent",
                                        operation = "recheck",
                                        torrent = %self.info_hash_hex,
                                        result = "error",
                                        error = %error,
                                        "recheck failed; stopping task"
                                    );
                                    break;
                                }
                                RecheckOutcome::Complete
                                | RecheckOutcome::Paused { .. }
                                | RecheckOutcome::Cancelled => {}
                            }
                            // Recheck may transition a complete torrent back
                            // to downloading when corruption is found.
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                        TorrentCmd::CancelJob { .. } => {}
                        TorrentCmd::Reannounce => {
                            self.tracker_event = TrackerEvent::Empty;
                            self.schedule_active_tracker_tier_now();
                        }
                        TorrentCmd::ReloadFilePolicy { reply } => {
                            let result = self.apply_file_policy_from_db().await;
                            if let Err(error) = &result {
                                warn!(
                                    component = "torrent",
                                    operation = "reload_file_policy",
                                    torrent = %self.info_hash_hex,
                                    result = "error",
                                    error = %error,
                                    "failed to reload persisted file policy; retaining the active policy"
                                );
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(result);
                            }
                        }
                        TorrentCmd::UpdateLimits { limits, reply } => {
                            let result = self.apply_torrent_limits(&limits).await;
                            if let Err(error) = &result {
                                warn!(
                                    component = "torrent",
                                    operation = "update_limits",
                                    torrent = %self.info_hash_hex,
                                    result = "error",
                                    error = %error,
                                    "failed to apply persisted torrent limits"
                                );
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(result);
                            }
                        }
                        TorrentCmd::UpdateTrackers { trackers, reply } => {
                            let result = self.apply_tracker_urls(trackers).await;
                            if let Err(error) = &result {
                                warn!(
                                    component = "torrent",
                                    operation = "update_trackers",
                                    torrent = %self.info_hash_hex,
                                    result = "error",
                                    error = %error,
                                    "failed to apply updated tracker URLs"
                                );
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(result);
                            }
                        }
                        TorrentCmd::UpdatePeerExchange(enabled) => {
                            self.pex_enabled = enabled;
                        }
                    }
                }

                Some(event) = self.peer_event_rx.recv() => {
                    if let Some(event) = peer_event_if_active(self.paused, event) {
                        self.handle_peer_event(event).await;
                    }
                    if self.crash_safety_work_pending() {
                        if !self.drive_crash_safety(&mut cmd_rx, &mut pending_command).await {
                            break;
                        }
                        reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                    }
                }

                Some(event) = self.peer_disconnect_rx.recv() => {
                    if let Some(event) = peer_event_if_active(self.paused, event) {
                        self.handle_peer_event(event).await;
                        if self.active_peers.is_empty() {
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                    }
                }

                completion = self.transfer_jobs.next(), if !self.transfer_jobs.is_empty() => {
                    if let Some(completion) = completion {
                        self.handle_transfer_completion(completion).await;
                    }
                }

                Some(event) = self.tracker_workers.recv_event() => {
                    match event {
                        TrackerWorkerEvent::Announce(result) => {
                            self.handle_tracker_result(result).await;
                        }
                        TrackerWorkerEvent::Scrape(scrape) => {
                            self.handle_tracker_scrape(scrape).await;
                        }
                    }
                }

                _ = choke_tick.tick() => {
                    self.run_choker().await;
                }

                _ = tracker_tick.tick() => {
                    self.evict_banned_peers().await;
                    self.enforce_seed_limits().await;
                    if self.transfer_stats_dirty {
                        self.persist_progress().await;
                    }
                    if !self.paused {
                        self.start_due_tracker_announces().await;
                    }
                    if self.crash_safety_work_pending() {
                        if !self.drive_crash_safety(&mut cmd_rx, &mut pending_command).await {
                            break;
                        }
                        reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                    }
                }

                _ = peer_retry_tick.tick() => {
                    if !self.paused {
                        self.retry_known_tracker_peers().await;
                    }
                }

                _ = peer_control_retry_tick.tick() => {
                    self.retry_pending_peer_controls();
                    self.retry_memory_denied_peer_requests().await;
                }

                _ = sleep_until_download_tokens(self.download_token_wake), if self.download_token_wake.is_some() => {
                    self.download_token_wake = None;
                    if !self.paused {
                        let peers: Vec<SocketAddr> = self.active_peers.keys().copied().collect();
                        for peer in peers {
                            self.refill_peer_requests(peer).await;
                        }
                    }
                }

                _ = &mut webseed_sleep, if !self.paused && self.active_peers.is_empty() && !self.meta.webseeds.is_empty() && !self.picker.is_complete() => {
                    // Webseed response bodies are already bounded, but the
                    // shared download bucket may be configured far below a
                    // protocol block size. Waiting for that bucket in the
                    // actor used to make Pause/Shutdown wait for hours. Keep
                    // the rate wait cancellable by the command receiver, but
                    // let disk handling finish once the budget was acquired.
                    let operation = self.download_next_webseed_block();
                    let outcome = {
                        tokio::pin!(operation);
                        tokio::select! {
                            result = &mut operation => Ok::<_, Option<TorrentCmd>>(result),
                            command = cmd_rx.recv() => Err(command),
                        }
                    };
                    match outcome {
                        Ok(Some(block)) => {
                            self.handle_block(block).await;
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                        Ok(None) => {
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                        Err(Some(command)) => {
                            // The canceled operation may have reserved a
                            // picker request before it reached its cleanup
                            // path. There are no active peers under this
                            // branch, so reset only the webseed-side request
                            // state before replaying the command through the
                            // normal actor path.
                            self.picker.reset_outstanding_requests();
                            pending_command = Some(command);
                            reset_webseed_sleep(&mut webseed_sleep, self.webseed_wake_delay());
                        }
                        Err(None) => {
                            warn!(
                                component = "torrent",
                                operation = "run",
                                torrent = %self.info_hash_hex,
                                result = "command_channel_closed",
                                "torrent command channel closed; shutting down"
                            );
                            self.cancel_tracker_announces();
                            self.announce_stopped_with_control_deadline().await;
                            self.drain_transfer_jobs().await;
                            self.persist_progress().await;
                            self.save_fastresume_for_exit().await;
                            self.shutdown_peers().await;
                            break;
                        }
                    }
                }
            }
        }

        // A Shutdown command wins FIFO ordering, but lifecycle/query commands
        // may already be queued behind it while the bounded cleanup above is
        // running. Resolve their replies before dropping the receiver so
        // callers observe a terminal error/result instead of a silent oneshot
        // disconnect.
        reject_pending_torrent_commands(&mut cmd_rx, &mut pending_command);
    }

    fn webseed_wake_delay(&self) -> Duration {
        if self.paused
            || !self.active_peers.is_empty()
            || self.meta.webseeds.is_empty()
            || self.picker.is_complete()
        {
            return WEBSEED_RETRY_MAX;
        }

        let now = Instant::now();
        let mut earliest_retry = WEBSEED_RETRY_MAX;
        let mut has_ready_seed = false;
        for (failures, next_attempt) in self
            .webseed_failures
            .iter()
            .zip(self.webseed_next_attempt.iter())
        {
            if *failures == u8::MAX {
                continue;
            }
            match next_attempt {
                Some(deadline) => {
                    earliest_retry = earliest_retry.min(deadline.saturating_duration_since(now));
                }
                None => has_ready_seed = true,
            }
        }
        if has_ready_seed {
            Duration::from_millis(100)
        } else {
            earliest_retry
        }
    }

    async fn start_due_tracker_announces(&mut self) {
        if self.tracker_tiers.is_empty() {
            return;
        }

        let tier_idx = self.active_tracker_tier.min(self.tracker_tiers.len() - 1);
        let available = self.tracker_workers.available();
        if available == 0 {
            return;
        }
        let candidates = tracker_keys_for_announce(
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
        if candidates.is_empty() {
            return;
        }

        let (uploaded, downloaded) = self.transfer_snapshot().await;
        let context = self.tracker_announce_context(uploaded, downloaded);
        self.tracker_workers.start(candidates, context);
    }

    async fn handle_tracker_scrape(&mut self, result: TrackerScrapeResult) {
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
        tracker.scrape_complete = Some(result.scrape.complete);
        tracker.scrape_incomplete = Some(result.scrape.incomplete);
        tracker.scrape_downloaded = Some(result.scrape.downloaded);
        self.persist_tracker_state().await;
    }

    async fn handle_tracker_result(&mut self, result: TrackerAnnounceResult) {
        self.tracker_workers.complete(result.key, result.generation);
        if !self.tracker_workers.is_current(result.generation) {
            return;
        }
        let (tier_idx, tracker_idx) = result.key;
        let Some(tier) = self.tracker_tiers.get_mut(tier_idx) else {
            return;
        };
        let Some(tracker) = tier.get_mut(tracker_idx) else {
            return;
        };
        let failed = result.response.is_err();
        match result.response {
            Ok(resp) => {
                let peers: Vec<SocketAddr> = resp.peers.iter().map(|peer| peer.addr).collect();
                tracker.on_success_with_min_interval(&resp, self.min_announce_interval);
                self.persist_tracker_state().await;
                self.tracker_event = tracker_event_after_success(self.tracker_event, result.event);
                if !peers.is_empty() && !self.paused {
                    self.remember_tracker_peers(&peers);
                    info!(
                        torrent = %self.info_hash_hex,
                        tracker = %url_log_target(&result.url),
                        peers = peers.len(),
                        "tracker announce returned peers"
                    );
                    self.connect_peers(peers, PeerSource::Tracker).await;
                }
            }
            Err(err) => {
                warn!(
                    component = "tracker",
                    operation = "announce",
                    torrent = %self.info_hash_hex,
                    tracker = %url_log_target(&result.url),
                    result = "error",
                    error = %err,
                    "tracker announce failed"
                );
                tracker.on_failure(err);
                self.persist_tracker_state().await;
            }
        }
        if self.meta.private {
            if failed {
                self.advance_private_tracker_after_failure((tier_idx, tracker_idx));
            }
        } else {
            self.maybe_advance_tracker_tier(tier_idx);
        }
    }

    fn tracker_announce_context(&self, uploaded: u64, downloaded: u64) -> TrackerAnnounceContext {
        TrackerAnnounceContext {
            info_hash: self.meta.info_hash,
            uploaded,
            downloaded,
            left: self.picker.bytes_left(),
            listen_port: match self.network_budget.listen_port() {
                0 => self.listen_port,
                port => port,
            },
            http_timeout: self.http_timeout,
            udp_timeout: self.udp_timeout,
            numwant: protocol_numwant(self.peer_capacity()),
            egress_policy: self.egress_policy,
            resources: self.resources.clone(),
        }
    }

    fn maybe_advance_tracker_tier(&mut self, tier_idx: usize) {
        if tier_idx != self.active_tracker_tier || self.tracker_tiers[tier_idx].is_empty() {
            return;
        }
        let has_inflight = self.tracker_workers.has_inflight_tier(tier_idx);
        if !has_inflight
            && self.tracker_tiers[tier_idx]
                .iter()
                .all(|tracker| matches!(&tracker.status, TrackerStatus::Error(_)))
        {
            self.advance_tracker_tier();
        }
    }

    fn advance_private_tracker_after_failure(&mut self, failed: TrackerKey) {
        if self.private_tracker_key != Some(failed) {
            return;
        }
        let Some(next) = next_tracker_key(&self.tracker_tiers, failed) else {
            return;
        };
        if next == failed {
            return;
        }

        self.cancel_tracker_announces();
        self.disconnect_private_tracker_peers();
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

    fn disconnect_private_tracker_peers(&mut self) {
        self.known_tracker_peers.clear();
        let peers = self.active_peers.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            self.evict_peer(peer);
        }
    }

    async fn announce_stopped(&mut self) {
        if !consume_stopped_announce(&mut self.stopped_announced) {
            return;
        }

        let (uploaded, downloaded) = self.transfer_snapshot().await;
        let context = self.tracker_announce_context(uploaded, downloaded);
        let candidates = tracker_keys_for_stopped_announce(
            &self.tracker_tiers,
            self.meta.private,
            self.private_tracker_key,
        )
        .into_iter()
        .filter_map(|(tier_idx, tracker_idx)| {
            let tracker = self.tracker_tiers.get(tier_idx)?.get(tracker_idx)?;
            Some((
                tier_idx,
                tracker_idx,
                tracker.url.clone(),
                tracker.tracker_id.clone(),
            ))
        })
        .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }

        // A stopped announce is still awaited so pause/quiesce/shutdown do
        // not report completion before the terminal tracker event has had a
        // chance to leave the process. The network work itself is bounded
        // and parallel, however: a dead tracker must not hold the actor for
        // one full timeout per tier entry. The outer deadline also cancels
        // requests whose individual HTTP/UDP timeout is longer.
        let mut pending = futures::stream::iter(candidates.into_iter().map(
            |(tier_idx, tracker_idx, url, tracker_id)| {
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
                    (tier_idx, tracker_idx, url, result)
                }
            },
        ))
        .buffer_unordered(MAX_TRACKER_ANNOUNCES_IN_FLIGHT);
        let deadline = sleep(STOPPED_TRACKER_ANNOUNCE_DEADLINE);
        tokio::pin!(deadline);
        let mut results = Vec::new();
        let mut deadline_exceeded = false;
        loop {
            tokio::select! {
                result = pending.next() => {
                    let Some(result) = result else { break };
                    results.push(result);
                }
                _ = &mut deadline => {
                    deadline_exceeded = true;
                    break;
                }
            }
        }
        if deadline_exceeded {
            warn!(
                component = "tracker",
                operation = "announce_stopped",
                torrent = %self.info_hash_hex,
                result = "deadline_exceeded",
                deadline_secs = STOPPED_TRACKER_ANNOUNCE_DEADLINE.as_secs(),
                completed = results.len(),
                "stopped tracker announces exceeded aggregate deadline"
            );
        }

        let had_results = !results.is_empty();
        for (tier_idx, tracker_idx, url, result) in results {
            let Some(tracker) = self
                .tracker_tiers
                .get_mut(tier_idx)
                .and_then(|tier| tier.get_mut(tracker_idx))
            else {
                continue;
            };
            match result {
                Ok(resp) => tracker.on_success_with_min_interval(&resp, self.min_announce_interval),
                Err(err) => {
                    warn!(
                        component = "tracker",
                        operation = "announce_stopped",
                        torrent = %self.info_hash_hex,
                        tracker = %url_log_target(&url),
                        result = "error",
                        error = %err,
                        "tracker stopped announce failed"
                    );
                    tracker.on_failure(err);
                }
            }
        }
        if had_results {
            self.persist_tracker_state().await;
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
                "stopped tracker announce exceeded the lifecycle control deadline"
            );
        }
    }

    fn restart_tracker_session(&mut self) {
        self.cancel_tracker_announces();
        self.tracker_event = TrackerEvent::Started;
        self.stopped_announced = false;
        // A stopped announce records the tracker's ordinary next interval.
        // Resuming is a new tracker session, so that old deadline must not
        // delay the Started event until the previous session's interval has
        // elapsed.
        self.schedule_trackers_now();
    }

    fn cancel_tracker_announces(&mut self) {
        self.tracker_workers.cancel();
    }

    async fn persist_tracker_state(&mut self) {
        if let Err(error) = self.persist_tracker_state_inner().await {
            warn!(
                component = "db",
                operation = "persist_tracker_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist tracker state; retaining the prior registry projection"
            );
        }
    }

    async fn persist_tracker_state_inner(&mut self) -> Result<(), String> {
        let (uploaded, downloaded) = self.transfer_snapshot().await;
        let left = db_i64(self.reported_bytes_left());
        let now = Instant::now();
        let mut rows = Vec::new();
        let mut tracker_index = 0i64;
        let mut tracker_count = 0u64;
        let mut tracker_snapshot_bytes = 0u64;
        // TorrentNG-client counterpart to the compatible-client service's
        // cached `t.message`
        // column: a torrent can be actively seeding/downloading fine while
        // its tracker rejects announces, which `state` alone never
        // reflects. Cache the first error/warning message found across
        // any tier on the registry entry so list/facet queries can read it
        // directly instead of needing a per-torrent round trip through
        // this actor.
        let mut tracker_message: Option<String> = None;
        for (tier_idx, tier) in self.tracker_tiers.iter().enumerate() {
            for tracker in tier {
                let failure_reason = tracker_failure_reason(&tracker.status);
                let warning_message = tracker_warning_message(&tracker.status);
                let row_bytes = 256u64
                    .saturating_add(self.info_hash_hex.len() as u64)
                    .saturating_add(tracker.url.len() as u64)
                    .saturating_add(tracker_status_label(&tracker.status).len() as u64)
                    .saturating_add(
                        tracker
                            .tracker_id
                            .as_ref()
                            .map_or(0, |tracker_id| tracker_id.len() as u64),
                    )
                    .saturating_add(
                        failure_reason
                            .as_ref()
                            .map_or(0, |message| message.len() as u64),
                    )
                    .saturating_add(
                        warning_message
                            .as_ref()
                            .map_or(0, |message| message.len() as u64),
                    );
                tracker_count = tracker_count.saturating_add(1);
                tracker_snapshot_bytes = tracker_snapshot_bytes.saturating_add(row_bytes);
                crate::engine::validate_tracker_snapshot_size(
                    tracker_count,
                    tracker_snapshot_bytes,
                )?;

                if tracker_message.is_none() {
                    tracker_message = failure_reason.clone().or_else(|| warning_message.clone());
                }
                rows.push(rt_db::TorrentTrackerRow {
                    info_hash: self.info_hash_hex.clone(),
                    tracker_index,
                    tier: i64::try_from(tier_idx).unwrap_or(i64::MAX),
                    url: tracker.url.clone(),
                    tracker_id: tracker.tracker_id.clone(),
                    status: tracker_status_label(&tracker.status).to_owned(),
                    last_announce_at: instant_to_unix(tracker.last_announce, now),
                    next_announce_at: instant_to_unix(tracker.next_announce, now),
                    last_success_at: instant_to_unix(tracker.last_success, now),
                    failure_reason,
                    warning_message,
                    seeders: tracker.scrape_complete.map(i64::from),
                    leechers: tracker.scrape_incomplete.map(i64::from),
                    completed: tracker.scrape_downloaded.map(i64::from),
                    uploaded: db_i64(uploaded),
                    downloaded: db_i64(downloaded),
                    left_bytes: left,
                });
                tracker_index += 1;
            }
        }
        let info_hash = self.info_hash_hex.clone();
        self.db
            .run("persist_tracker_state", move |db| {
                rt_db::replace_torrent_trackers(db, &info_hash, &rows)
                    .map_err(|error| error.to_string())
            })
            .await?;
        let mut reg = self.registry.write().await;
        if let Some(mut entry) = reg.get_mut(&self.info_hash_hex) {
            entry.tracker_message = tracker_message;
        };
        Ok(())
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

    async fn apply_tracker_urls(&mut self, trackers: Vec<String>) -> Result<(), String> {
        let tracker_tiers = tracker_tiers_from_urls(&trackers);
        self.refresh_tracker_state_memory(&tracker_tiers, tracker_tiers.capacity())?;
        self.cancel_tracker_announces();
        if self.meta.private {
            self.disconnect_private_tracker_peers();
        }
        self.tracker_tiers = tracker_tiers;
        self.private_tracker_key = if self.meta.private {
            first_tracker_key(&self.tracker_tiers)
        } else {
            None
        };
        self.active_tracker_tier = self.private_tracker_key.map_or(0, |(tier, _)| tier);
        self.tracker_event = TrackerEvent::Empty;
        self.schedule_trackers_now();

        {
            let mut registry = self.registry.write().await;
            if let Some(mut entry) = registry.get_mut(&self.info_hash_hex) {
                entry.tracker_message = None;
            };
        }
        // The engine persists the compact URL override and fresh detail rows
        // before sending this command. A tracker result that was already
        // waiting on the DB worker can nevertheless complete after that
        // transaction and overwrite the detail rows with its older snapshot.
        // Re-persist the projection after installing the new URLs so the
        // command acknowledgement also fences that stale detail write.
        self.persist_tracker_state().await;
        Ok(())
    }

    fn schedule_active_tracker_tier_now(&mut self) {
        if self.tracker_tiers.is_empty() {
            return;
        }
        let tier_idx = self.active_tracker_tier.min(self.tracker_tiers.len() - 1);
        for (selected_tier, tracker_idx) in tracker_keys_for_announce(
            &self.tracker_tiers,
            tier_idx,
            self.meta.private,
            self.private_tracker_key,
        ) {
            if let Some(tracker) = self
                .tracker_tiers
                .get_mut(selected_tier)
                .and_then(|tier| tier.get_mut(tracker_idx))
            {
                tracker.schedule_immediate();
            }
        }
    }

    async fn apply_file_policy_from_db(&mut self) -> Result<(), String> {
        let info_hash = self.info_hash_hex.clone();
        let (rows, limits) = self
            .db
            .run("load_file_policy", move |db| {
                let rows = rt_db::list_torrent_files(db, &info_hash)
                    .map_err(|error| format!("loading torrent file policy: {error}"))?;
                let limits = Self::torrent_limits_from_db(db, &info_hash)?;
                Ok((rows, limits))
            })
            .await?;
        // Admission must precede the clone and replacement map construction;
        // reserving only after those allocations lets a large durable policy
        // briefly bypass the metadata governor during reload.
        let _file_policy_workspace_memory_lease = self.reserve_file_policy_workspace_memory()?;
        let mut effective_files = self.metainfo_files.clone();
        let mut policy = HashMap::with_capacity(rows.len());
        for row in rows {
            let file_index = u32::try_from(row.file_index).map_err(|_| {
                format!(
                    "persisted file policy has invalid file index {}",
                    row.file_index
                )
            })?;
            let Some(file) = effective_files
                .iter_mut()
                .find(|file| file.index == file_index)
            else {
                return Err(format!(
                    "persisted file policy references unknown file index {file_index}"
                ));
            };
            file.path = parse_persisted_file_path(&row.path)?;
            if !(0..=2).contains(&row.priority) {
                return Err(format!(
                    "persisted file policy has invalid priority {} for file {file_index}",
                    row.priority
                ));
            }
            if policy
                .insert(
                    file_index,
                    (
                        row.wanted && !file.pad,
                        if file.pad { 0 } else { row.priority },
                    ),
                )
                .is_some()
            {
                return Err(format!(
                    "persisted file policy contains duplicate file index {file_index}"
                ));
            }
        }

        // Padding entries are synthetic zero bytes. Keep them disabled even
        // when a partial durable projection omitted the row.
        for file in &effective_files {
            policy
                .entry(file.index)
                .or_insert((!file.pad, if file.pad { 0 } else { 1 }));
        }

        crate::engine::validate_file_path_projection(
            effective_files
                .iter()
                .filter(|file| !file.pad)
                .map(|file| file.path.as_display()),
        )?;

        let paths_changed = self.meta.files.len() != effective_files.len()
            || self
                .meta
                .files
                .iter()
                .zip(&effective_files)
                .any(|(current, effective)| {
                    current.index != effective.index || current.path != effective.path
                });
        if paths_changed {
            let mut changed_file_indices = self
                .meta
                .files
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
                .map_err(|error| format!("invalidating renamed file pieces: {error}"))?;
            let file_policy_memory_lease = self.reserve_file_policy_memory(&effective_files)?;
            // Peer upload contexts hold an Arc<PieceMap> and a copy of the
            // storage root. End those sessions before publishing the new map,
            // otherwise an in-flight peer can keep reading the old path after
            // the rename has been accepted.
            let piece_map = build_piece_map(self.meta.piece_length, &effective_files)?;
            self.shutdown_peers().await;
            // A path projection change changes the bytes represented by every
            // overlapping piece: the old file may still exist, the new file
            // may be missing, or an operator may have moved a replacement
            // into place. Do not retain the old picker result across that
            // boundary, or the task can remain falsely complete/seeding and
            // serve data from an unverified path.
            for (first, last) in changed_piece_ranges {
                for piece in first..last {
                    self.picker.reject_piece(piece as usize);
                    self.restored_partial_pieces.remove(&piece);
                }
            }
            Arc::make_mut(&mut self.meta).files = effective_files;
            self.piece_map = piece_map;
            self._file_policy_memory_lease = file_policy_memory_lease;
            lock_prepared_files(&self.prepared_files).clear();
        }

        let piece_count = self.piece_map.piece_count as usize;
        let file_lengths: HashMap<u32, u64> = self
            .meta
            .files
            .iter()
            .map(|file| (file.index, file.length))
            .collect();
        let mut enabled = vec![false; piece_count];
        let mut priority = vec![false; piece_count];
        for piece in 0..self.piece_map.piece_count {
            let regions = self
                .piece_map
                .piece_to_file_regions(piece)
                .map_err(|error| format!("building file policy for piece {piece}: {error}"))?;
            let mut any_wanted = false;
            let mut any_high = false;
            let mut any_first_last = false;
            for region in regions {
                if region.pad {
                    continue;
                }
                let (wanted, priority) =
                    policy.get(&region.file_index).copied().unwrap_or((true, 1));
                any_wanted |= wanted && priority > 0;
                any_high |= wanted && priority > 1;
                let file_len = file_lengths
                    .get(&region.file_index)
                    .copied()
                    .ok_or_else(|| {
                        format!(
                            "piece {piece} references unknown file index {}",
                            region.file_index
                        )
                    })?;
                if limits.first_last_piece_prio && wanted && priority > 0 {
                    if region.file_offset > file_len
                        || region.length > file_len.saturating_sub(region.file_offset)
                    {
                        return Err(format!(
                            "piece {piece} has an out-of-range region for file {}",
                            region.file_index
                        ));
                    }
                    any_first_last |= region.file_offset == 0
                        || region.file_offset.saturating_add(region.length) >= file_len;
                }
            }
            enabled[piece as usize] = any_wanted;
            if any_high || any_first_last {
                priority[piece as usize] = true;
            }
        }
        self.apply_piece_policy(&enabled, priority);
        Ok(())
    }

    fn apply_piece_policy(&mut self, enabled: &[bool], priority: Vec<bool>) {
        for (piece, enabled) in enabled.iter().copied().enumerate() {
            self.picker.set_piece_enabled(piece, enabled);
        }
        self.picker.set_priority_mask(priority);
    }

    async fn apply_torrent_limits_from_db(&mut self) -> Result<(), String> {
        let info_hash = self.info_hash_hex.clone();
        let limits = self
            .db
            .run("load_torrent_limits", move |db| {
                Self::torrent_limits_from_db(db, &info_hash)
            })
            .await?;
        self.apply_torrent_limits(&limits).await
    }

    async fn apply_torrent_limits(&mut self, limits: &EngineTorrentLimits) -> Result<(), String> {
        // Validate and install the complete file policy before changing any
        // other in-memory limit. This keeps a malformed durable file-policy
        // row from producing a partially applied runtime configuration.
        self.apply_file_policy_from_db().await?;
        self.picker.set_sequential(limits.sequential_download);
        if let Some(piece) = limits.sequential_download_from_piece {
            self.picker.set_sequential_from_piece(piece as usize);
        }
        self.super_seeding = limits.super_seeding;
        self.seed_ratio_limit = limits
            .seed_ratio_limit
            .filter(|value| value.is_finite() && *value >= 0.0);
        self.seed_idle_limit = limits
            .seed_idle_limit
            .and_then(|minutes| u64::try_from(minutes).ok())
            .filter(|minutes| *minutes > 0)
            .map(|minutes| Duration::from_secs(minutes.saturating_mul(60)));
        self.torrent_max_peers = limits.max_connections.and_then(|value| {
            usize::try_from(value)
                .ok()
                .filter(|connections| *connections > 0)
        });
        self.set_torrent_download_limit(limits.download_limit);
        self.set_torrent_upload_limit(limits.upload_limit);
        Ok(())
    }

    fn fail_closed_runtime_policy(&mut self, operation: &str, error: &str) {
        for piece in 0..self.piece_map.piece_count {
            self.picker.set_piece_enabled(piece as usize, false);
        }
        self.picker.set_priority(Vec::new());
        self.paused = true;
        self.recheck_restore_state = Some(TorrentState::Paused);
        warn!(
            component = "torrent",
            operation,
            torrent = %self.info_hash_hex,
            result = "paused",
            error,
            "persisted runtime policy could not be loaded; torrent starts paused with piece selection disabled"
        );
    }

    fn peer_capacity(&self) -> usize {
        self.torrent_max_peers
            .map(|limit| limit.min(self.max_peers))
            .unwrap_or(self.max_peers)
    }

    fn set_torrent_download_limit(&mut self, limit: Option<i64>) {
        self.torrent_download_limit_bytes_per_sec =
            limit.and_then(|value| (value > 0).then_some(value as u64));
        self.recompute_download_limit();
    }

    fn recompute_download_limit(&mut self) {
        // The process-wide limiter is shared by every task. Including the
        // global value here would divide the global allowance once per
        // torrent and make the configured limit unusably strict at scale.
        self.download_limit_bytes_per_sec = self.torrent_download_limit_bytes_per_sec;
        self.download_tokens_updated = Instant::now();
        self.download_tokens = self
            .download_limit_bytes_per_sec
            .map(download_bucket_capacity)
            .unwrap_or(u64::MAX);
    }

    fn set_torrent_upload_limit(&mut self, limit: Option<i64>) {
        self.torrent_upload_limit_bytes_per_sec =
            limit.and_then(|value| (value > 0).then_some(value as u64));
        self.recompute_upload_limit();
    }

    fn recompute_upload_limit(&mut self) {
        // See recompute_download_limit: global traffic is enforced by the
        // engine-owned shared bucket. The per-torrent bucket is shared by all
        // peers; changing it wakes any peer waiting on the old rate.
        self.torrent_upload
            .set_limit(self.torrent_upload_limit_bytes_per_sec);
    }

    fn torrent_limits_from_db(
        db: &Connection,
        info_hash: &str,
    ) -> Result<EngineTorrentLimits, String> {
        match rt_db::get_torrent_limits(db, info_hash) {
            Ok(row) => Ok(EngineTorrentLimits {
                download_limit: row.download_limit,
                upload_limit: row.upload_limit,
                max_connections: row.max_connections,
                seed_ratio_limit: row.seed_ratio_limit,
                seed_idle_limit: row.seed_idle_limit,
                sequential_download: row.sequential_download,
                sequential_download_from_piece: row.sequential_download_from_piece,
                first_last_piece_prio: row.first_last_piece_prio,
                force_start: row.force_start,
                super_seeding: row.super_seeding,
                auto_tmm: row.auto_tmm,
                auto_management: row.auto_management,
            }),
            Err(rt_db::DbError::NotFound(_)) => Ok(EngineTorrentLimits::default()),
            Err(error) => Err(format!("loading torrent limits: {error}")),
        }
    }

    fn advance_tracker_tier(&mut self) {
        if self.tracker_tiers.len() <= 1 {
            return;
        }
        let old = self.active_tracker_tier;
        self.active_tracker_tier = (self.active_tracker_tier + 1) % self.tracker_tiers.len();
        for tracker in &mut self.tracker_tiers[self.active_tracker_tier] {
            tracker.schedule_immediate();
        }
        warn!(
            torrent = %self.info_hash_hex,
            from_tier = old,
            to_tier = self.active_tracker_tier,
            "advancing tracker tier after announce failures"
        );
    }

    async fn prepare_resume(&mut self) -> Result<(), String> {
        let was_paused = self.paused;
        self.paused = false;
        self.restart_tracker_session();
        if let Err(error) = self.set_state_checked(TorrentState::Checking).await {
            self.paused = was_paused;
            if was_paused {
                self.cancel_tracker_announces();
            }
            return Err(format!("failed to persist torrent resume state: {error}"));
        }
        self.recheck_restore_state = None;
        Ok(())
    }

    #[cfg(test)]
    async fn run_recheck(&mut self, job_id: Option<String>) -> RecheckOutcome {
        let (_, replacement_cmd_rx) = mpsc::channel(1);
        let mut cmd_rx = std::mem::replace(&mut self.cmd_rx, replacement_cmd_rx);
        let mut pending_command = None;
        let outcome = self
            .run_recheck_with_receiver(&mut cmd_rx, &mut pending_command, job_id)
            .await;
        self.cmd_rx = cmd_rx;
        outcome
    }

    async fn run_recheck_with_receiver(
        &mut self,
        cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
        pending_command: &mut Option<TorrentCmd>,
        job_id: Option<String>,
    ) -> RecheckOutcome {
        // A recheck decides the torrent's completion itself, so a completion
        // gate or queued verification from before it is void.
        self.completion_gate = None;
        self.completion_verify_plan = None;
        let outcome = self
            .run_recheck_inner(cmd_rx, pending_command, job_id)
            .await;
        self.completion_verifying = false;
        self.recheck_subset = None;
        if !self.paused
            && matches!(
                &outcome,
                RecheckOutcome::Complete | RecheckOutcome::Cancelled
            )
        {
            self.connect_deferred_priority_peers().await;
        }
        outcome
    }

    async fn run_recheck_inner(
        &mut self,
        cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
        pending_command: &mut Option<TorrentCmd>,
        mut job_id: Option<String>,
    ) -> RecheckOutcome {
        let _frames = rt_storage::frame::global_frame_pool().verification_scope();
        let mut valid = 0usize;
        // A subset recheck (completion `sample` verification) re-hashes only
        // the chosen pieces and leaves every other piece as it was.
        let subset = self.recheck_subset.take();
        let verification_run = self.completion_verifying;
        let previously_completed = self.registry_completed_at().await.is_some();
        let recheck_trigger = if verification_run {
            "completion_verify"
        } else if self.recovery_recheck_reason.take().is_some() {
            "crash_recovery"
        } else {
            "recheck"
        };
        let mut verified_bytes = 0_u64;
        // The picker is updated as each piece is verified, so retaining a
        // second result entry for every piece only multiplied recheck memory
        // for large torrents. Keep the job-facing invalid sample bounded at
        // the same limit enforced by the database, while the accumulator
        // retains the complete count for progress events.
        let mut invalid_pieces = RecheckInvalidPieces::new(self.piece_map.piece_count);
        // A recheck supersedes any announce already in flight. Without a
        // generation bump, a stale Started/Completed response can be handled
        // after a pause or cancellation and mutate the tracker session that
        // the recheck is supposed to have frozen.
        self.cancel_tracker_announces();
        // `shutdown_peers` also clears in-memory piece assemblies. Flush
        // those received blocks first so starting a recheck cannot discard
        // progress that has not yet crossed the normal complete-piece write
        // boundary.
        self.drain_transfer_jobs().await;
        self.save_fastresume(false).await;
        self.shutdown_peers().await;
        // Do not checkpoint the old fastresume bitmap while this scan is in
        // progress. Publish each piece below only after its current bytes
        // have been hashed, so a crash cannot restore stale-valid pieces.
        if subset.is_none() {
            for piece in 0..self.piece_map.piece_count {
                self.picker.reject_piece(piece as usize);
            }
        }
        if let Err(error) = self.set_state_checked(TorrentState::Checking).await {
            self.paused = true;
            self.cancel_tracker_announces();
            await_stopped_announce_or_queue_command(self, cmd_rx, pending_command).await;
            if let Some(job_id) = &job_id {
                self.persist_recheck_job_progress(
                    job_id,
                    0,
                    verified_bytes,
                    &invalid_pieces,
                    JOB_STATE_FAILED,
                    Some("failed to persist torrent checking state"),
                )
                .await;
            }
            return RecheckOutcome::Failed(format!(
                "failed to persist torrent checking state: {error}"
            ));
        }

        let mut subset_pieces = subset.as_ref().map(|pieces| pieces.iter().copied());
        let mut next_piece = 0u32;
        loop {
            let piece = match subset_pieces.as_mut() {
                Some(pieces) => match pieces.next() {
                    Some(piece) => piece,
                    None => break,
                },
                None => {
                    if next_piece >= self.piece_map.piece_count {
                        break;
                    }
                    let piece = next_piece;
                    next_piece += 1;
                    piece
                }
            };
            match self
                .pending_recheck_control(cmd_rx, pending_command, &mut job_id)
                .await
            {
                Some(RecheckOutcome::Paused {
                    reply,
                    previous_paused,
                    previous_restore_state,
                    state_persisted,
                }) => {
                    self.save_fastresume(false).await;
                    let result = if state_persisted {
                        Ok(())
                    } else {
                        self.set_state_checked(TorrentState::Paused)
                            .await
                            .map_err(|error| {
                                format!("failed to persist torrent pause after recheck: {error}")
                            })
                    };
                    if let Err(error) = &result {
                        warn!(
                            component = "torrent",
                            operation = "pause_during_recheck",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to persist torrent pause after recheck"
                        );
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result.clone());
                    }
                    if result.is_err() {
                        // The pause request was rejected by durable storage.
                        // Restore the pre-request runtime intent and keep the
                        // recheck running against its still-Checking state;
                        // otherwise the task would remain paused while the
                        // registry/database still claim verification is active.
                        self.paused = previous_paused;
                        self.recheck_restore_state = previous_restore_state;
                        continue;
                    }
                    if let Some(job_id) = &job_id {
                        self.persist_recheck_job_progress(
                            job_id,
                            piece,
                            verified_bytes,
                            &invalid_pieces,
                            if result.is_ok() {
                                JOB_STATE_PAUSED
                            } else {
                                JOB_STATE_FAILED
                            },
                            if result.is_ok() {
                                Some("recheck paused")
                            } else {
                                Some("failed to persist torrent pause after recheck")
                            },
                        )
                        .await;
                    }
                    await_stopped_announce_or_queue_command(self, cmd_rx, pending_command).await;
                    return RecheckOutcome::Paused {
                        reply: None,
                        previous_paused,
                        previous_restore_state,
                        state_persisted: true,
                    };
                }
                Some(RecheckOutcome::Cancelled) => {
                    let restore_state = self.recheck_restore_state.unwrap_or(TorrentState::Paused);
                    self.paused =
                        matches!(restore_state, TorrentState::Paused | TorrentState::Stopped);
                    self.recheck_restore_state = Some(restore_state);
                    self.cancel_tracker_announces();
                    self.save_fastresume(false).await;
                    await_stopped_announce_or_queue_command(self, cmd_rx, pending_command).await;
                    let lifecycle_persisted = if let Err(error) =
                        self.set_state_checked(restore_state).await
                    {
                        warn!(
                            component = "torrent",
                            operation = "cancel_recheck",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to persist torrent lifecycle state after recheck cancellation"
                        );
                        false
                    } else {
                        true
                    };
                    if lifecycle_persisted {
                        if let Some(job_id) = &job_id {
                            self.persist_recheck_job_progress(
                                job_id,
                                piece,
                                verified_bytes,
                                &invalid_pieces,
                                JOB_STATE_CANCELLED,
                                Some("recheck cancelled"),
                            )
                            .await;
                        }
                    } else {
                        // Keep the durable job in `cancelling` when the
                        // restored torrent state was not committed. A
                        // terminal job would be ignored by restart recovery
                        // while the torrent row still says `Checking`.
                        warn!(
                            component = "torrent",
                            operation = "cancel_recheck",
                            torrent = %self.info_hash_hex,
                            result = "recoverable",
                            "recheck cancellation remains recoverable until torrent state persistence succeeds"
                        );
                    }
                    return RecheckOutcome::Cancelled;
                }
                Some(RecheckOutcome::Shutdown) => {
                    // A disconnected command channel reaches this branch
                    // without the explicit Shutdown arm below. Keep the
                    // terminal tracker event best effort for both cases.
                    self.announce_stopped_with_control_deadline().await;
                    self.save_fastresume_for_exit().await;
                    if let Err(error) = self.set_state_checked(TorrentState::Stopped).await {
                        warn!(
                            component = "torrent",
                            operation = "shutdown_during_recheck",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to persist stopped state after recheck shutdown"
                        );
                    }
                    if let Some(job_id) = &job_id {
                        self.persist_recheck_job_progress(
                            job_id,
                            piece,
                            verified_bytes,
                            &invalid_pieces,
                            JOB_STATE_PAUSED,
                            Some("recheck interrupted by shutdown"),
                        )
                        .await;
                    }
                    return RecheckOutcome::Shutdown;
                }
                Some(RecheckOutcome::Complete) | None => {}
                Some(RecheckOutcome::Failed(error)) => {
                    return RecheckOutcome::Failed(error);
                }
            }

            let result = PieceVerifier::new(
                &self.save_root,
                &self.storage,
                &self.piece_map,
                &self.meta.pieces,
            )
            .verify_piece(piece)
            .await;
            match result {
                VerifyResult::Valid => {
                    valid += 1;
                    verified_bytes = verified_bytes.saturating_add(
                        self.piece_length(piece).map(u64::from).unwrap_or_default(),
                    );
                    self.picker.mark_have(piece as usize);
                }
                VerifyResult::Invalid => {
                    invalid_pieces.record(piece);
                    self.picker.reject_piece(piece as usize);
                }
                VerifyResult::Missing { .. } => {
                    invalid_pieces.record(piece);
                    self.picker.reject_piece(piece as usize);
                }
            }

            if piece > 0 && piece % 64 == 0 {
                self.save_fastresume(false).await;
                if let Some(job_id) = &job_id {
                    self.persist_recheck_job_progress(
                        job_id,
                        piece + 1,
                        verified_bytes,
                        &invalid_pieces,
                        JOB_STATE_RUNNING,
                        Some("recheck progress"),
                    )
                    .await;
                }
            }
        }

        info!(
            torrent = %self.info_hash_hex,
            valid,
            invalid = invalid_pieces.total,
            "recheck complete"
        );
        // From here the recheck's own verdict (Seeding or Downloading) is
        // authoritative; the one-byte completion hold no longer applies.
        self.completion_verifying = false;
        self.report_recheck_outcome(
            &invalid_pieces,
            valid,
            previously_completed,
            verification_run,
            subset.is_some(),
            recheck_trigger,
        )
        .await;
        self.save_fastresume(subset.is_none()).await;

        let final_state = if self.paused {
            if let Some(restore_state) = self.recheck_restore_state {
                // A recheck requested for a paused or stopped torrent must
                // not silently start it. Dormant tasks are also constructed
                // paused, so this branch is gated by the explicit restore
                // intent rather than by `self.paused` alone.
                self.tracker_event = TrackerEvent::Started;
                self.cancel_tracker_announces();
                restore_state
            } else {
                // Promotion temporarily starts an active dormant torrent in
                // the paused runtime mode while its recheck command is being
                // delivered. Once verification completes, restore activity.
                self.paused = false;
                self.restart_tracker_session();
                if self.picker.is_complete() {
                    self.tracker_event = TrackerEvent::Completed;
                    self.schedule_trackers_now();
                    TorrentState::Seeding
                } else {
                    self.persist_progress().await;
                    TorrentState::Downloading
                }
            }
        } else if self.picker.is_complete() {
            self.tracker_event = TrackerEvent::Completed;
            self.schedule_trackers_now();
            TorrentState::Seeding
        } else {
            self.tracker_event = tracker_event_after_incomplete_recheck(self.tracker_event);
            // Rechecking cancelled any announce from the previous state. The
            // tracker needs a fresh left/count snapshot when the torrent is
            // found incomplete, even if the old session deadline was still in
            // the future.
            self.schedule_trackers_now();
            self.persist_progress().await;
            TorrentState::Downloading
        };
        if let Err(error) = self.set_state_checked(final_state).await {
            self.paused = true;
            self.tracker_event = TrackerEvent::Started;
            self.cancel_tracker_announces();
            await_stopped_announce_or_queue_command(self, cmd_rx, pending_command).await;
            self.shutdown_peers().await;
            if let Some(job_id) = &job_id {
                self.persist_recheck_job_progress(
                    job_id,
                    self.piece_map.piece_count,
                    verified_bytes,
                    &invalid_pieces,
                    JOB_STATE_FAILED,
                    Some("failed to persist torrent state after recheck"),
                )
                .await;
            }
            return RecheckOutcome::Failed(format!(
                "failed to persist torrent state after recheck: {error}"
            ));
        }
        if let Some(job_id) = &job_id {
            self.persist_recheck_job_progress(
                job_id,
                self.piece_map.piece_count,
                verified_bytes,
                &invalid_pieces,
                JOB_STATE_COMPLETED,
                Some("recheck completed"),
            )
            .await;
        }
        RecheckOutcome::Complete
    }

    async fn pending_recheck_control(
        &mut self,
        cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
        pending_command: &mut Option<TorrentCmd>,
        active_job_id: &mut Option<String>,
    ) -> Option<RecheckOutcome> {
        loop {
            match cmd_rx.try_recv() {
                Ok(TorrentCmd::Pause { reply }) => {
                    let previous_paused = self.paused;
                    let previous_restore_state = self.recheck_restore_state;
                    self.paused = true;
                    self.recheck_restore_state = Some(TorrentState::Paused);
                    self.cancel_tracker_announces();
                    self.shutdown_peers().await;
                    return Some(RecheckOutcome::Paused {
                        reply,
                        previous_paused,
                        previous_restore_state,
                        state_persisted: false,
                    });
                }
                Ok(TorrentCmd::Shutdown) => {
                    self.paused = true;
                    self.cancel_tracker_announces();
                    self.shutdown_peers().await;
                    self.announce_stopped_with_control_deadline().await;
                    return Some(RecheckOutcome::Shutdown);
                }
                Ok(TorrentCmd::Resume { reply }) => {
                    self.paused = false;
                    self.recheck_restore_state = None;
                    // A paused torrent normally has no tracker workers left;
                    // rechecking it keeps the actor inside this control path,
                    // so the regular `prepare_resume` helper is not reached
                    // when the job is resumed here.
                    self.restart_tracker_session();
                    if let Some(reply) = reply {
                        // Recheck already owns the durable `Checking` state;
                        // acknowledge the control request without waiting for
                        // the current verification pass to finish.
                        let _ = reply.send(Ok(()));
                    }
                }
                Ok(TorrentCmd::Reannounce) => {
                    self.schedule_active_tracker_tier_now();
                }
                Ok(TorrentCmd::ReloadFilePolicy { reply }) => {
                    let result = self.apply_file_policy_from_db().await;
                    if let Err(error) = &result {
                        warn!(
                            component = "torrent",
                            operation = "reload_file_policy",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to reload persisted file policy during recheck; retaining the active policy"
                        );
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                Ok(TorrentCmd::UpdateLimits { limits, reply }) => {
                    let result = self.apply_torrent_limits(&limits).await;
                    if let Err(error) = &result {
                        warn!(
                            component = "torrent",
                            operation = "update_limits",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to apply persisted torrent limits during recheck"
                        );
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                Ok(TorrentCmd::UpdateTrackers { trackers, reply }) => {
                    let result = self.apply_tracker_urls(trackers).await;
                    if let Err(error) = &result {
                        warn!(
                            component = "torrent",
                            operation = "update_trackers",
                            torrent = %self.info_hash_hex,
                            result = "error",
                            error = %error,
                            "failed to apply updated tracker URLs during recheck"
                        );
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
                Ok(TorrentCmd::UpdatePeerExchange(enabled)) => {
                    self.pex_enabled = enabled;
                }
                Ok(TorrentCmd::Recheck { job_id }) => {
                    if let Some(job_id) = job_id {
                        if active_job_id.is_none() {
                            // Startup/resume rechecks normally have no durable
                            // job. If an explicit recheck is queued while one
                            // of those scans is still running, adopt its job
                            // identity so the scan completes the job instead
                            // of silently discarding the command.
                            *active_job_id = Some(job_id);
                        } else if active_job_id.as_deref() != Some(job_id.as_str()) {
                            debug!(
                                torrent = %self.info_hash_hex,
                                active_job_id = ?active_job_id,
                                queued_job_id = %job_id,
                                "ignoring recheck command for a different active job"
                            );
                        }
                    }
                }
                Ok(TorrentCmd::CancelJob { job_id }) => {
                    if active_job_id.as_deref() != Some(job_id.as_str()) {
                        debug!(
                            torrent = %self.info_hash_hex,
                            job_id,
                            active_job_id = ?active_job_id,
                            "ignoring cancellation for an inactive recheck job"
                        );
                        continue;
                    }
                    debug!(
                        torrent = %self.info_hash_hex,
                        job_id,
                        "cancelling recheck job"
                    );
                    return Some(RecheckOutcome::Cancelled);
                }
                Ok(TorrentCmd::NewPeers(_)) => {}
                Ok(TorrentCmd::PriorityPeers(peers)) => {
                    self.defer_priority_peers(peers);
                }
                Ok(TorrentCmd::BanPeer(peer)) => {
                    self.evict_peer(peer);
                }
                Ok(TorrentCmd::EvictBannedPeers) => {
                    self.evict_banned_peers().await;
                }
                Ok(TorrentCmd::GetPeers { reply }) => {
                    let _ = reply.send(Vec::new());
                }
                Ok(TorrentCmd::GetPeerSnapshotCount { reply }) => {
                    let _ = reply.send(0);
                }
                Ok(TorrentCmd::GetPeerSnapshots { reply, .. }) => {
                    let _ = reply.send(Ok(Vec::new()));
                }
                Ok(TorrentCmd::GetWebseeds { reply }) => {
                    let _ = reply.send(self.webseed_snapshots());
                }
                Ok(TorrentCmd::GetRuntimeStats { reply }) => {
                    let _ = reply.send(self.runtime_stats());
                }
                Ok(TorrentCmd::GetPeerSummary { reply }) => {
                    let _ = reply.send(self.peer_stats());
                }
                Ok(TorrentCmd::AcceptPeer { .. }) => {}
                Ok(TorrentCmd::AcceptUtpPeer { .. }) => {}
                Ok(TorrentCmd::QuiesceForStorageMove { reply }) => {
                    let was_paused = self.paused;
                    self.paused = true;
                    self.cancel_tracker_announces();
                    self.shutdown_peers().await;
                    if let Err(error) = self.set_state_checked(TorrentState::Paused).await {
                        self.paused = was_paused;
                        if !was_paused {
                            self.restart_tracker_session();
                        }
                        let _ = reply.send(Err(format!(
                            "failed to persist torrent quiesce state: {error}"
                        )));
                        continue;
                    }
                    let _ = reply.send(Ok(was_paused));
                    await_stopped_announce_or_queue_command(self, cmd_rx, pending_command).await;
                    return Some(RecheckOutcome::Paused {
                        reply: None,
                        previous_paused: was_paused,
                        previous_restore_state: self.recheck_restore_state,
                        state_persisted: true,
                    });
                }
                Ok(TorrentCmd::ResumeAfterStorageMove {
                    new_save_root,
                    resume_paused,
                    reply,
                }) => {
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
                        lock_prepared_files(&self.prepared_files).clear();
                    }
                    self.paused = resume_paused;
                    self.recheck_restore_state = resume_paused.then_some(TorrentState::Paused);
                    if !resume_paused {
                        self.restart_tracker_session();
                    }
                    let _ = reply.send(Ok(()));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return None,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Some(RecheckOutcome::Shutdown);
                }
            }
        }
    }

    async fn restore_fastresume(&mut self) -> bool {
        let mut state = match self.fastresume.load(&self.info_hash_hex) {
            Ok(state) => state,
            Err(rt_fastresume::FastresumeError::NotFound) => {
                debug!(
                    component = "fastresume",
                    operation = "load",
                    torrent = %self.info_hash_hex,
                    result = "not_found",
                    "no fastresume state"
                );
                return false;
            }
            Err(e) => {
                warn!(
                    component = "fastresume",
                    operation = "load",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %e,
                    "failed to load fastresume state"
                );
                return false;
            }
        };

        if let Err(e) = state.validate(&self.meta.info_hash, self.meta.pieces.len() as u32) {
            warn!(
                component = "fastresume",
                operation = "validate",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %e,
                "discarding incompatible fastresume state"
            );
            return false;
        }

        let settings = self.crash_safety.settings();
        let location = self.crash_safety.for_location(&self.save_root);
        let previous_run = self.crash_safety.previous_run();
        let saved_this_run = state.durability.saved_by_run_id != 0
            && state.durability.saved_by_run_id == self.crash_safety.run_id();
        let state_was_clean = state.clean_shutdown;
        if previous_run.may_be_host_crash() && !saved_this_run {
            let inputs = RecoveryInputs {
                previous_run,
                crash_reference_unix: self.crash_safety.assessment().crash_reference_unix,
                now_unix: crash_safety::unix_seconds(),
                policy: crash_safety::recovery_policy(location.resolved.host_crash_recovery),
                recent_window_secs: settings.recent_write_window_secs,
                mount_trust: location.trust,
                weak_mount_escalation: settings.weak_mount_escalation,
                state_synced: state.durability.synced,
                last_data_write_unix: state.durability.last_data_write_unix,
                completed_unix: self.registry_completed_at().await,
                state_saved_this_run: saved_this_run,
            };
            if let RecoveryAction::Recheck(reason) = decide_recovery(&inputs) {
                warn!(
                    component = "fastresume",
                    operation = "crash_recovery",
                    torrent = %self.info_hash_hex,
                    previous_run = previous_run.as_str(),
                    reason = reason.as_str(),
                    mount_trust = location.trust.as_str(),
                    "discarding resume state after a possible host crash; the torrent will be rechecked"
                );
                self.crash_safety.record_recovery_recheck(reason);
                self.recovery_recheck_reason = Some(reason);
                self.append_torrent_event(
                    "crash_recovery_recheck",
                    "resume state discarded after an unclean host shutdown; rechecking",
                    serde_json::json!({
                        "level": "warn",
                        "reason": reason.as_str(),
                        "previous_run": previous_run.as_str(),
                        "mount_trust": location.trust.as_str(),
                    }),
                )
                .await;
                return false;
            }
        }

        if !state.clean_shutdown {
            match state.apply_unclean_shutdown_watermark() {
                Some(downgraded) => {
                    warn!(
                        torrent = %self.info_hash_hex,
                        downgraded,
                        "fastresume had unclean shutdown; applying bounded dirty-piece recheck watermark"
                    );
                }
                None => {
                    warn!(
                        torrent = %self.info_hash_hex,
                        "discarding unclean fastresume state without durability watermark"
                    );
                    return false;
                }
            }
        }

        let hints = collect_file_hints(&self.save_root, &self.meta);
        let invalidated = state.apply_file_hints(hints, &self.piece_map);
        if invalidated > 0 {
            warn!(
                torrent = %self.info_hash_hex,
                invalidated,
                "fastresume file hints changed"
            );
        }

        if should_audit_allocation(
            crash_safety::audit_policy(location.resolved.structural_audit),
            previous_run,
            state_was_clean,
            saved_this_run,
        ) {
            let (downgraded, unsupported_files) = self.audit_allocation(&mut state).await;
            self.crash_safety
                .record_audit(u64::from(downgraded), unsupported_files);
            if downgraded > 0 {
                warn!(
                    component = "fastresume",
                    operation = "allocation_audit",
                    torrent = %self.info_hash_hex,
                    downgraded,
                    "pieces overlapping unwritten or hole extents were downgraded for re-verification"
                );
                self.append_torrent_event(
                    "allocation_audit_downgraded",
                    "pieces overlapping unwritten or hole extents will be re-verified",
                    serde_json::json!({ "level": "warn", "pieces": downgraded }),
                )
                .await;
            }
        }
        self.last_data_write_unix.store(
            state.durability.last_data_write_unix,
            std::sync::atomic::Ordering::Relaxed,
        );

        for (piece, piece_state) in state.pieces.iter().copied().enumerate() {
            match piece_state {
                PieceState::Valid => self.picker.mark_have(piece),
                PieceState::Invalid | PieceState::Missing | PieceState::Unknown => {
                    self.picker.reject_piece(piece)
                }
            }
        }
        for partial in &state.partial_pieces {
            self.picker
                .restore_partial_piece(partial.piece as usize, &partial.received_blocks);
            if self
                .picker
                .partial_pieces()
                .iter()
                .any(|(piece, _)| *piece == partial.piece)
            {
                self.restored_partial_pieces.insert(partial.piece);
            }
        }

        {
            let mut reg = self.registry.write().await;
            if let Some(mut entry) = reg.get_mut(&self.info_hash_hex) {
                entry.stats.uploaded = state.uploaded_bytes;
                entry.stats.downloaded = state.downloaded_bytes;
                entry.total_length = self.meta.total_length();
                entry.amount_left = self.reported_bytes_left();
                entry.finalizing = self.is_finalizing();
            };
        }

        if state.file_hints.is_empty() {
            self.save_fastresume(false).await;
        }

        info!(
            torrent = %self.info_hash_hex,
            valid = state.valid_piece_count(),
            unknown = state.unknown_piece_count(),
            "fastresume restored"
        );
        true
    }

    async fn write_block(&mut self, block: &BlockEvent) -> anyhow::Result<()> {
        self.write_block_inner(block, None).await
    }

    pub(super) fn transfer_io_context(&self) -> TransferIoContext {
        TransferIoContext {
            meta: Arc::clone(&self.meta),
            piece_map: Arc::clone(&self.piece_map),
            save_root: self.save_root.clone(),
            storage: self.storage.clone(),
            prepared_files: Arc::clone(&self.prepared_files),
            prepared_file_locks: Arc::clone(&self.prepared_file_locks),
        }
    }

    async fn write_leased_block(&mut self, event: &LeasedBlockEvent) -> anyhow::Result<()> {
        self.write_block_inner(&event.block, event._memory_lease.clone())
            .await
    }

    async fn write_block_inner(
        &mut self,
        block: &BlockEvent,
        shared_payload_lease: Option<Arc<MemoryLease>>,
    ) -> anyhow::Result<()> {
        let block_len = u32::try_from(block.data.len())
            .map_err(|_| anyhow::anyhow!("peer block length does not fit in u32"))?;
        self.piece_map.validate_request_block(BlockExtent::new(
            block.piece,
            block.offset,
            block_len,
        ))?;
        let regions = self
            .piece_map
            .validate_request(block.piece, block.offset, block_len)?;
        let mut data_offset = 0usize;

        for region in regions {
            let region_len = usize::try_from(region.length)
                .map_err(|_| anyhow::anyhow!("piece region length does not fit in memory"))?;
            let start = data_offset;
            let end = start
                .checked_add(region_len)
                .ok_or_else(|| anyhow::anyhow!("piece region data offset overflow"))?;
            let region_data = block
                .data
                .get(start..end)
                .ok_or_else(|| anyhow::anyhow!("peer block does not cover mapped piece regions"))?;
            if region.pad {
                if region_data.iter().any(|byte| *byte != 0) {
                    anyhow::bail!("peer block contains non-zero bytes for BEP 47 padding");
                }
                data_offset = end;
                continue;
            }
            let file = self
                .meta
                .files
                .iter()
                .find(|file| file.index == region.file_index)
                .ok_or_else(|| anyhow::anyhow!("file index {} out of range", region.file_index))?;
            let path = file.path.resolve(&self.save_root);
            self.prepare_file_once(file.index, &path, file.length)
                .await?;
            data_offset = end;
            match shared_payload_lease.as_ref() {
                Some(lease) => {
                    scheduled_write_with_shared_payload_lease(
                        &self.storage,
                        IoClass::PeerWrite,
                        &path,
                        region.file_offset,
                        block.data.slice(start..end),
                        true,
                        Arc::clone(lease),
                    )
                    .await?;
                }
                None => {
                    scheduled_write(
                        &self.storage,
                        IoClass::PeerWrite,
                        &path,
                        region.file_offset,
                        bytes::Bytes::copy_from_slice(region_data),
                        true,
                    )
                    .await?;
                }
            }
        }

        if data_offset != block.data.len() {
            anyhow::bail!("mapped piece regions do not cover the peer block");
        }
        self.note_data_write();
        Ok(())
    }

    async fn write_completed_piece(&mut self, piece: u32) -> anyhow::Result<()> {
        let (data, payload_lease) = self
            .piece_assemblies
            .get(&piece)
            .filter(|assembly| assembly.is_complete())
            .and_then(|assembly| {
                Some((
                    assembly.completed_data.as_ref()?.clone(),
                    assembly._memory_lease.clone(),
                ))
            })
            .ok_or_else(|| anyhow::anyhow!("piece {piece} is not fully assembled"))?;
        let regions = self.piece_map.piece_to_file_regions(piece)?;
        for region in regions {
            if region.pad {
                continue;
            }
            let file = self
                .meta
                .files
                .iter()
                .find(|file| file.index == region.file_index)
                .ok_or_else(|| anyhow::anyhow!("file index {} out of range", region.file_index))?;
            let path = file.path.resolve(&self.save_root);
            self.prepare_file_once(file.index, &path, file.length)
                .await?;
            let start = usize::try_from(region.piece_offset)
                .map_err(|_| anyhow::anyhow!("piece region offset does not fit in memory"))?;
            let region_len = usize::try_from(region.length)
                .map_err(|_| anyhow::anyhow!("piece region length does not fit in memory"))?;
            let end = start
                .checked_add(region_len)
                .ok_or_else(|| anyhow::anyhow!("piece region data offset overflow"))?;
            anyhow::ensure!(
                data.get(start..end).is_some(),
                "piece assembly does not cover mapped region"
            );
            let region_data = data.slice(start..end);
            match payload_lease.as_ref() {
                Some(lease) => {
                    scheduled_write_with_shared_payload_lease(
                        &self.storage,
                        IoClass::PeerWrite,
                        &path,
                        region.file_offset,
                        region_data,
                        true,
                        Arc::clone(lease),
                    )
                    .await?;
                }
                None => {
                    scheduled_write(
                        &self.storage,
                        IoClass::PeerWrite,
                        &path,
                        region.file_offset,
                        region_data,
                        true,
                    )
                    .await?;
                }
            }
        }
        self.note_data_write();
        Ok(())
    }

    async fn prepare_file_once(
        &mut self,
        file_index: u32,
        path: &std::path::Path,
        len: u64,
    ) -> anyhow::Result<()> {
        prepare_file_once_with_locks(
            &self.storage,
            &self.prepared_files,
            &self.prepared_file_locks,
            file_index,
            path,
            len,
        )
        .await?;
        Ok(())
    }

    async fn verify_piece(&self, piece: u32) -> VerifyResult {
        let verifier = PieceVerifier::new(
            &self.save_root,
            &self.storage,
            &self.piece_map,
            &self.meta.pieces,
        );
        match &self.hybrid_v2 {
            Some(meta) => match meta.piece_expectation(piece) {
                Some(expectation) => verifier.verify_hybrid_piece(piece, &expectation).await,
                None => VerifyResult::Missing {
                    file_index: 0,
                    reason: format!("no authenticated v2 file range for hybrid piece {piece}"),
                },
            },
            None => verifier.verify_piece(piece).await,
        }
    }

    async fn verify_completed_piece(&mut self, piece: u32) -> VerifyResult {
        if let Some(assembly) = self
            .piece_assemblies
            .get(&piece)
            .filter(|assembly| assembly.is_complete())
        {
            self.completed_piece_verify_from_memory =
                self.completed_piece_verify_from_memory.saturating_add(1);
            let Some(expected) = self.meta.pieces.get(piece as usize) else {
                return VerifyResult::Missing {
                    file_index: 0,
                    reason: format!("no hash for piece {piece}"),
                };
            };
            let Some(data) = assembly.completed_data.as_ref().cloned() else {
                return VerifyResult::Missing {
                    file_index: 0,
                    reason: format!("completed piece {piece} has no immutable payload"),
                };
            };
            let payload_lease = assembly._memory_lease.clone();
            if let Some(hybrid_v2) = &self.hybrid_v2 {
                let Some(expectation) = hybrid_v2.piece_expectation(piece) else {
                    return VerifyResult::Missing {
                        file_index: 0,
                        reason: format!("no authenticated v2 file range for hybrid piece {piece}"),
                    };
                };
                let Ok(piece_length) = usize::try_from(self.meta.piece_length) else {
                    return VerifyResult::Missing {
                        file_index: expectation.file_index,
                        reason: "hybrid piece length does not fit memory address space".to_owned(),
                    };
                };
                let Ok(v2_content_len) = usize::try_from(expectation.content_len) else {
                    return VerifyResult::Missing {
                        file_index: expectation.file_index,
                        reason: "hybrid content length does not fit memory address space"
                            .to_owned(),
                    };
                };
                let hashes = match payload_lease.as_ref() {
                    Some(lease) => {
                        self.storage
                            .hash_hybrid_piece_retry_queue_full_with_shared_payload_lease(
                                data,
                                v2_content_len,
                                piece_length,
                                Arc::clone(lease),
                            )
                            .await
                    }
                    None => {
                        self.storage
                            .hash_hybrid_piece_retry_queue_full(data, v2_content_len, piece_length)
                            .await
                    }
                };
                return match hashes {
                    Ok(hashes) if hashes.v1_sha1 == *expected => {
                        let actual_v2 = match expectation.piece_leaf_count {
                            Some(_) => hashes.v2_piece_root,
                            None => hashes.v2_file_root,
                        };
                        if actual_v2 == expectation.expected_root {
                            return VerifyResult::Valid;
                        }
                        VerifyResult::Invalid
                    }
                    Ok(_) => VerifyResult::Invalid,
                    Err(error) => VerifyResult::Missing {
                        file_index: expectation.file_index,
                        reason: error.to_string(),
                    },
                };
            }
            let hash = match payload_lease.as_ref() {
                Some(lease) => {
                    self.storage
                        .hash_sha1_retry_queue_full_with_shared_payload_lease(
                            data,
                            Arc::clone(lease),
                        )
                        .await
                }
                None => self.storage.hash_sha1_retry_queue_full(data).await,
            };
            match hash {
                Ok(actual) if &actual == expected => return VerifyResult::Valid,
                Ok(_) => return VerifyResult::Invalid,
                Err(e) => {
                    return VerifyResult::Missing {
                        file_index: 0,
                        reason: e.to_string(),
                    }
                }
            }
        }
        self.completed_piece_verify_from_disk =
            self.completed_piece_verify_from_disk.saturating_add(1);
        self.verify_piece(piece).await
    }

    fn piece_length(&self, piece: u32) -> anyhow::Result<u32> {
        if piece as usize >= self.meta.pieces.len() {
            anyhow::bail!("piece {piece} out of range");
        }
        let last = self.meta.pieces.len().saturating_sub(1) as u32;
        if piece == last {
            let total = self.meta.total_length();
            let rem = total % self.meta.piece_length;
            Ok(if rem == 0 {
                self.meta.piece_length as u32
            } else {
                rem as u32
            })
        } else {
            Ok(self.meta.piece_length as u32)
        }
    }

    async fn set_state_checked(&mut self, state: TorrentState) -> Result<(), String> {
        let (previous, row, state_event) = {
            let mut reg = self.registry.write().await;
            let Some(mut entry) = reg.get_mut(&self.info_hash_hex) else {
                return Err(format!(
                    "torrent {} is missing from the registry",
                    self.info_hash_hex
                ));
            };
            let previous = entry.clone();
            let previous_state = entry.state;
            if let Err(error) = entry.transition(state) {
                warn!(
                    component = "torrent",
                    operation = "transition_state",
                    torrent = %self.info_hash_hex,
                    state = %state,
                    result = "rejected",
                    error = %error,
                    "rejected invalid torrent state transition"
                );
                return Err(error.to_string());
            }
            entry.total_length = self.meta.total_length();
            entry.amount_left = self.reported_bytes_left();
            entry.finalizing = self.is_finalizing();
            if state == TorrentState::Seeding && entry.completed_at.is_none() {
                entry.amount_left = 0;
                entry.finalizing = false;
                entry.completed_at = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
            let row = crate::engine::row_from_v1_meta(&entry, &self.meta);
            let state_event = (previous_state != state).then(|| rt_db::SessionEventRow {
                event_id: None,
                occurred_at: db_i64(unix_now()),
                info_hash: Some(self.info_hash_hex.clone()),
                kind: "torrent_state_changed".to_owned(),
                message: Some("torrent runtime state changed".to_owned()),
                payload: serde_json::json!({
                    "from": previous_state.as_str(),
                    "to": state.as_str(),
                    "total_length": entry.total_length,
                    "amount_left": entry.amount_left,
                })
                .to_string(),
            });
            (previous, row, state_event)
        };
        let retention = self.event_retention;
        let persistence = self
            .db
            .run("persist_torrent_state", move |db| {
                let tx = db.transaction().map_err(|error| error.to_string())?;
                let updated =
                    rt_db::update_runtime_in_tx(&tx, &row).map_err(|error| error.to_string())?;
                if !updated {
                    return Err(format!(
                        "torrent {} is missing from the database",
                        row.info_hash
                    ));
                }
                if let Some(event) = state_event.as_ref() {
                    rt_db::append_session_event_in_tx(&tx, event)
                        .map_err(|error| error.to_string())?;
                    rt_db::prune_session_events_in_tx(&tx, retention)
                        .map_err(|error| error.to_string())?;
                }
                tx.commit().map_err(|error| error.to_string())
            })
            .await;
        if let Err(error) = persistence {
            if let Some(mut entry) = self.registry.write().await.get_mut(&self.info_hash_hex) {
                restore_runtime_projection(&mut entry, &previous);
            }
            warn!(
                component = "db",
                operation = "persist_torrent_state",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist torrent state"
            );
            return Err(error);
        } else {
            // The timer is runtime state derived from the durable state. Do
            // not start/stop it before the transition and its database row
            // have both committed: a rejected transition or failed write
            // must leave seed-limit enforcement observing the old state.
            if state == TorrentState::Seeding {
                self.seeding_started_at.get_or_insert_with(Instant::now);
            } else {
                self.seeding_started_at = None;
            }
            self.transfer_stats_dirty = false;
        }
        Ok(())
    }

    async fn persist_progress(&mut self) {
        let (previous, row) = {
            let mut reg = self.registry.write().await;
            let Some(mut entry) = reg.get_mut(&self.info_hash_hex) else {
                return;
            };
            let previous = entry.clone();
            entry.total_length = self.meta.total_length();
            entry.amount_left = self.reported_bytes_left();
            entry.finalizing = self.is_finalizing();
            let row = crate::engine::row_from_v1_meta(&entry, &self.meta);
            (previous, row)
        };
        let persistence = self
            .db
            // `run_batched`: the worker may commit this alongside other
            // torrents' queued progress writes in one shared transaction
            // instead of one fsync'd commit per torrent per tick — this is
            // the actual write-throughput ceiling at high torrent counts,
            // since every `TorrentTask` ticks and persists independently.
            .run_batched("persist_torrent_progress", move |tx| {
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
                restore_progress_projection(&mut entry, &previous);
            }
            warn!(
                component = "db",
                operation = "persist_torrent_progress",
                torrent = %self.info_hash_hex,
                result = "error",
                error = %error,
                "failed to persist torrent progress"
            );
        } else {
            self.transfer_stats_dirty = false;
        }
    }

    async fn persist_progress_throttled(&mut self, force: bool) {
        const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(5);
        let now = Instant::now();
        if !force {
            match self.last_progress_persist {
                Some(last) if now.duration_since(last) < PROGRESS_PERSIST_INTERVAL => return,
                None => {
                    // Start the checkpoint interval on the first partial
                    // completion. Persisting here would make the transfer
                    // actor await a database commit and payload barrier
                    // before it can process the next peer command or block.
                    self.last_progress_persist = Some(now);
                    return;
                }
                Some(_) => {}
            }
        }
        self.last_progress_persist = Some(now);
        self.persist_progress().await;
        self.save_fastresume(false).await;
    }

    async fn persist_recheck_job_progress(
        &mut self,
        job_id: &str,
        next_piece: u32,
        verified_bytes: u64,
        invalid_pieces: &RecheckInvalidPieces,
        state: &str,
        message: Option<&str>,
    ) {
        let now = unix_now();
        let done = i64::from(next_piece.min(self.piece_map.piece_count));
        let byte_offset = self.verified_byte_offset(next_piece);
        let verified_bytes = db_i64(verified_bytes);
        let job_id = job_id.to_owned();
        let state = state.to_owned();
        let message = message.map(str::to_owned);
        let invalid_piece_count = invalid_pieces.total;
        let invalid_pieces = invalid_pieces.values.clone();
        let invalid_pieces_truncated = invalid_piece_count > rt_db::MAX_JOB_INVALID_PIECES;
        let log_job_id = job_id.clone();
        let log_state = state.clone();
        let persistence = self
            .db
            .run("persist_recheck_progress", move |db| {
                let mut job = rt_db::get_job(db, &job_id).map_err(|error| error.to_string())?;
                // A cancellation/failure/completion written by the engine
                // control path wins over any progress message already in
                // flight. Without this fence, a late actor write could
                // resurrect a terminal recheck job on the next snapshot.
                if job.finished_at.is_some()
                    || matches!(
                        job.state.as_str(),
                        JOB_STATE_CANCELLED | JOB_STATE_COMPLETED | JOB_STATE_FAILED
                    )
                {
                    return Ok(());
                }
                if job.state == JOB_STATE_CANCELLING
                    && state != JOB_STATE_CANCELLED
                    && state != JOB_STATE_COMPLETED
                {
                    // The engine has durably recorded a cancellation intent.
                    // Do not let a progress or completion write resurrect
                    // the job while the actor is still unwinding the scan.
                    return Ok(());
                }
                let cancellation_completed_after_request =
                    job.state == JOB_STATE_CANCELLING && state == JOB_STATE_COMPLETED;
                let state = if cancellation_completed_after_request {
                    JOB_STATE_CANCELLED.to_owned()
                } else {
                    state
                };
                let message = if cancellation_completed_after_request {
                    Some("recheck cancellation arrived after verification completed".to_owned())
                } else {
                    message
                };
                job.state = state.clone();
                job.done = done;
                job.checkpoint = done;
                job.piece_index = Some(done);
                job.byte_offset = Some(byte_offset);
                job.verified_bytes = verified_bytes;
                job.invalid_pieces = invalid_pieces.clone();
                job.updated_at = db_i64(now);
                if matches!(
                    state.as_str(),
                    JOB_STATE_CANCELLED | JOB_STATE_COMPLETED | JOB_STATE_FAILED
                ) {
                    job.finished_at = Some(db_i64(now));
                }
                let event = rt_db::JobEventRow {
                    event_id: None,
                    job_id: job_id.clone(),
                    occurred_at: db_i64(now),
                    kind: match state.as_str() {
                        JOB_STATE_CANCELLED => "check_cancelled",
                        JOB_STATE_COMPLETED => "check_completed",
                        JOB_STATE_FAILED => "check_failed",
                        _ => "check_progress",
                    }
                    .to_owned(),
                    message,
                    payload: serde_json::json!({
                        "piece_index": job.piece_index,
                        "verified_bytes": job.verified_bytes,
                        "invalid_piece_count": invalid_piece_count,
                        "invalid_pieces_truncated": invalid_pieces_truncated,
                        "invalid_pieces": job.invalid_pieces,
                        "state": state,
                    })
                    .to_string(),
                };
                let tx = db.transaction().map_err(|error| error.to_string())?;
                rt_db::upsert_job_in_tx(&tx, &job).map_err(|error| error.to_string())?;
                rt_db::append_job_event_in_tx(&tx, &event).map_err(|error| error.to_string())?;
                tx.commit().map_err(|error| error.to_string())
            })
            .await;
        if let Err(e) = persistence {
            warn!(
                component = "db",
                operation = "persist_recheck_progress_and_event",
                job_id = %log_job_id,
                state = %log_state,
                result = "error",
                error = %e,
                "failed to persist recheck progress and event atomically"
            );
        }
    }

    fn verified_byte_offset(&self, next_piece: u32) -> i64 {
        let bytes = (next_piece as u64).saturating_mul(self.meta.piece_length);
        db_i64(bytes.min(self.meta.total_length()))
    }

    /// Bytes left as shown to trackers and API consumers. A finished download
    /// still waiting for durability or read-back verification reports one byte,
    /// so nothing treats it as complete before its data is safe.
    fn reported_bytes_left(&self) -> u64 {
        let pending_verification_bytes = self
            .pending_transfer_pieces
            .iter()
            .filter(|(_, work)| {
                work.verify_ready || work.verify_in_flight || work.aggregate_write_ready
            })
            .filter_map(|(&piece, _)| self.piece_length(piece).ok())
            .map(u64::from)
            .fold(0u64, u64::saturating_add);
        let left = self
            .picker
            .bytes_left()
            .saturating_add(pending_verification_bytes);
        if left == 0 && self.is_finalizing() {
            1
        } else {
            left
        }
    }

    /// The download is complete on disk but held until its data is durable
    /// and any read-back verification has passed. Published to the registry as
    /// `TorrentEntry::finalizing` next to every `amount_left` update.
    fn is_finalizing(&self) -> bool {
        self.picker.bytes_left() == 0
            && (self.completion_gate.is_some() || self.completion_verifying)
    }

    fn note_data_write(&self) {
        self.last_data_write_unix.store(
            crash_safety::unix_seconds(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    async fn registry_completed_at(&mut self) -> Option<u64> {
        let registry = Arc::clone(&self.registry);
        let info_hash_hex = self.info_hash_hex.clone();
        let completed_at = registry
            .read()
            .await
            .get(&info_hash_hex)
            .and_then(|entry| entry.completed_at);
        completed_at
    }

    /// Append a per-torrent event to the session event log. Best effort: a
    /// failed append is logged and never affects the torrent.
    async fn append_torrent_event(
        &mut self,
        kind: &'static str,
        message: &str,
        payload: serde_json::Value,
    ) {
        let event = rt_db::SessionEventRow {
            event_id: None,
            occurred_at: db_i64(unix_now()),
            info_hash: Some(self.info_hash_hex.clone()),
            kind: kind.to_owned(),
            message: Some(message.to_owned()),
            payload: payload.to_string(),
        };
        let retention = self.event_retention;
        let result = self
            .db
            .run("append_torrent_event", move |db| {
                let tx = db.transaction().map_err(|error| error.to_string())?;
                rt_db::append_session_event_in_tx(&tx, &event)
                    .map_err(|error| error.to_string())?;
                rt_db::prune_session_events_in_tx(&tx, retention)
                    .map_err(|error| error.to_string())?;
                tx.commit().map_err(|error| error.to_string())
            })
            .await;
        if let Err(error) = result {
            debug!(
                component = "db",
                operation = "append_torrent_event",
                torrent = %self.info_hash_hex,
                kind,
                error = %error,
                "could not record torrent event"
            );
        }
    }

    /// The last piece just verified: decide how the download completes.
    ///
    /// With the completion gate (default) the torrent is held at "one byte
    /// left" until the payload is durable and any configured read-back
    /// verification has passed; only then does it become `Seeding`, tell the
    /// tracker `completed`, and stamp `completed_at`.
    async fn on_download_complete(&mut self) {
        let location = self.crash_safety.for_location(&self.save_root);
        if !location.resolved.completion_gate
            && location.completion_verify == rt_config::CompletionVerifyMode::Off
        {
            // Gate disabled by the operator for this location: the historical
            // behavior.
            self.persist_progress_throttled(true).await;
            self.save_fastresume(false).await;
            self.finish_completion().await;
            return;
        }
        if self.completion_gate.is_none() {
            self.completion_gate = Some(CompletionGate::new(Arc::clone(&self.crash_safety)));
            // Publish the one-byte-remaining projection before anything slow.
            self.persist_progress().await;
        }
        if let GateStep::Verify(plan) = self.advance_completion_gate().await {
            self.completion_verify_plan = Some(plan);
        }
    }

    /// Try to satisfy the completion gate: run the payload barrier, then
    /// either release the completion or hand off to read-back verification.
    async fn advance_completion_gate(&mut self) -> GateStep {
        if !self.picker.is_complete() {
            // A piece was rejected (recheck, failed verification): the
            // download is not finished after all.
            self.completion_gate = None;
            self.persist_progress().await;
            return GateStep::Dropped;
        }
        let attempts = match self.completion_gate.as_mut() {
            None => return GateStep::Dropped,
            Some(gate) if Instant::now() < gate.next_attempt => return GateStep::Pending,
            Some(gate) => {
                gate.attempts += 1;
                gate.attempts
            }
        };
        if attempts > 1 {
            self.crash_safety.record_gate_retry();
        }
        let outcome = self.save_fastresume_with(false, true).await;
        if !outcome.releases_completion() {
            if let Some(gate) = self.completion_gate.as_mut() {
                gate.schedule_retry();
            }
            warn!(
                component = "torrent",
                operation = "completion_gate",
                torrent = %self.info_hash_hex,
                attempts,
                barrier = ?outcome.barrier,
                persisted = outcome.persisted,
                "download complete but its data is not yet durable; holding completion and retrying"
            );
            if attempts == 1 {
                self.append_torrent_event(
                    "completion_deferred",
                    "download complete but not yet durable; completion is held until storage confirms",
                    serde_json::json!({
                        "level": "warn",
                        "barrier": format!("{:?}", outcome.barrier),
                        "fastresume_persisted": outcome.persisted,
                    }),
                )
                .await;
            }
            return GateStep::Pending;
        }

        let location = self.crash_safety.for_location(&self.save_root);
        match location.completion_verify {
            rt_config::CompletionVerifyMode::Off => {
                self.finish_completion().await;
                GateStep::Released
            }
            rt_config::CompletionVerifyMode::Sample => GateStep::Verify(VerifyPlan::Sample),
            rt_config::CompletionVerifyMode::Full => GateStep::Verify(VerifyPlan::Full),
        }
    }

    /// Release a completed download: tell the tracker and become `Seeding`.
    async fn finish_completion(&mut self) {
        if let Some(gate) = self.completion_gate.take() {
            let attempts = gate.attempts;
            let waited_ms = gate.started.elapsed().as_millis() as u64;
            gate.release();
            if attempts > 1 {
                self.append_torrent_event(
                    "completion_released",
                    "completion released after storage confirmed the data is durable",
                    serde_json::json!({ "attempts": attempts, "waited_ms": waited_ms }),
                )
                .await;
            }
        }
        self.tracker_event = TrackerEvent::Completed;
        self.schedule_trackers_now();
        match self.set_state_checked(TorrentState::Seeding).await {
            Ok(()) => info!(
                component = "torrent",
                operation = "complete_download",
                torrent = %self.info_hash_hex,
                result = "ok",
                "download complete"
            ),
            Err(error) => {
                // `set_state_checked` rolled the registry back to Downloading
                // when its durable write failed. Keep the runtime on that same
                // active state; marking only the actor as paused would leave
                // the public projection claiming downloading while no task work
                // was possible.
                self.paused = false;
                self.recheck_restore_state = None;
                self.restart_tracker_session();
                // Completion can be reached while `drain_transfer_jobs` is
                // already handling a completion. Box this recursive async
                // edge so the rollback actor retains its full drain-before-
                // shutdown behavior without requiring an infinitely sized
                // future type.
                Box::pin(self.shutdown_peers()).await;
                warn!(
                    component = "torrent",
                    operation = "complete_download",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to persist seeding state; retaining downloading state"
                );
            }
        }
    }

    fn crash_safety_work_pending(&self) -> bool {
        self.completion_gate.is_some()
            || self.completion_verify_plan.is_some()
            || self.escalate_full_recheck
    }

    /// Advance queued crash-safety work that needs the command receiver: the
    /// completion gate's retry, the read-back verification it hands off to, and
    /// a full recheck escalated by a failed sample. Returns `false` when the
    /// task must stop (shutdown or an unrecoverable recheck failure).
    async fn drive_crash_safety(
        &mut self,
        cmd_rx: &mut mpsc::Receiver<TorrentCmd>,
        pending_command: &mut Option<TorrentCmd>,
    ) -> bool {
        if self.paused {
            return true;
        }
        if self.completion_gate.is_some() {
            if let GateStep::Verify(plan) = self.advance_completion_gate().await {
                self.completion_verify_plan = Some(plan);
            }
        }
        if let Some(plan) = self.completion_verify_plan.take() {
            self.begin_completion_verification(plan).await;
        } else if self.escalate_full_recheck {
            self.escalate_full_recheck = false;
        } else {
            return true;
        }
        match self
            .run_recheck_with_receiver(cmd_rx, pending_command, None)
            .await
        {
            RecheckOutcome::Shutdown => false,
            RecheckOutcome::Failed(error) => {
                warn!(
                    component = "torrent",
                    operation = "completion_verify",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %error,
                    "verification recheck failed; stopping task"
                );
                false
            }
            RecheckOutcome::Complete
            | RecheckOutcome::Paused { .. }
            | RecheckOutcome::Cancelled => true,
        }
    }

    /// Set up the recheck that reads a finished download back from disk.
    async fn begin_completion_verification(&mut self, plan: VerifyPlan) {
        if let Some(gate) = self.completion_gate.take() {
            gate.release();
        }
        let sample_percent = self
            .crash_safety
            .for_location(&self.save_root)
            .resolved
            .completion_verify_sample_percent;
        self.recheck_subset = match plan {
            VerifyPlan::Full => None,
            VerifyPlan::Sample => {
                let files: Vec<(u32, u64, bool)> = self
                    .meta
                    .files
                    .iter()
                    .map(|file| (file.index, file.length, file.pad))
                    .collect();
                let mut seed_bytes = [0u8; 8];
                for (slot, byte) in seed_bytes.iter_mut().zip(self.meta.info_hash.iter()) {
                    *slot = *byte;
                }
                select_completion_sample(
                    &self.piece_map,
                    &files,
                    sample_percent,
                    u64::from_le_bytes(seed_bytes),
                )
            }
        };
        self.completion_verifying = true;
        // Best effort: make the read-back come from the device, not RAM.
        let mut dropped = 0usize;
        for file in &self.meta.files {
            let path = file.path.resolve(&self.save_root);
            if self.storage.drop_page_cache(&path).await {
                dropped += 1;
            }
        }
        debug!(
            component = "torrent",
            operation = "completion_verify",
            torrent = %self.info_hash_hex,
            plan = ?plan,
            sampled_pieces = self.recheck_subset.as_ref().map(Vec::len),
            page_cache_dropped_files = dropped,
            "starting read-back verification of the completed download"
        );
    }

    /// Record what a recheck found: completion-verification results and, for a
    /// torrent that had already been reported complete, an integrity
    /// regression (with the affected paths so an operator or automation can
    /// act on copies made before the damage was known).
    async fn report_recheck_outcome(
        &mut self,
        invalid: &RecheckInvalidPieces,
        valid: usize,
        previously_completed: bool,
        verification_run: bool,
        sampled: bool,
        trigger: &'static str,
    ) {
        let checked = valid.saturating_add(invalid.total);
        if verification_run {
            self.crash_safety
                .record_completion_verify(checked as u64, invalid.total as u64);
            if invalid.total == 0 {
                self.append_torrent_event(
                    "completion_verified",
                    "completed download was read back from disk and verified",
                    serde_json::json!({
                        "mode": if sampled { "sample" } else { "full" },
                        "pieces_checked": checked,
                    }),
                )
                .await;
            } else {
                // Storage returned data that does not match what was written.
                // A failed sample is evidence enough to check everything.
                self.escalate_full_recheck = sampled;
                self.append_torrent_event(
                    "completion_verify_failed",
                    "read-back verification found data that does not match; the bad pieces will be downloaded again",
                    serde_json::json!({
                        "level": "error",
                        "mode": if sampled { "sample" } else { "full" },
                        "pieces_checked": checked,
                        "invalid_pieces": invalid.total,
                    }),
                )
                .await;
            }
        }
        if previously_completed && invalid.total > 0 {
            let sample: Vec<i64> = invalid
                .values
                .iter()
                .copied()
                .take(MAX_REGRESSION_SAMPLE_PIECES)
                .collect();
            let mut files: Vec<String> = Vec::new();
            for piece in &sample {
                let Ok(regions) = self.piece_map.piece_to_file_regions(*piece as u32) else {
                    continue;
                };
                for region in regions {
                    let Some(file) = self
                        .meta
                        .files
                        .iter()
                        .find(|file| file.index == region.file_index)
                    else {
                        continue;
                    };
                    let path = file.path.resolve(&self.save_root).display().to_string();
                    if files.len() < MAX_REGRESSION_FILES && !files.contains(&path) {
                        files.push(path);
                    }
                }
            }
            self.crash_safety.record_integrity_regression();
            warn!(
                component = "torrent",
                operation = "integrity_regression",
                torrent = %self.info_hash_hex,
                trigger,
                invalid_pieces = invalid.total,
                "a torrent already reported complete has pieces that no longer match; repairing in place"
            );
            self.append_torrent_event(
                "integrity_regression",
                "a completed torrent has pieces that no longer verify; missing data will be downloaded again in place",
                serde_json::json!({
                    "level": "error",
                    "trigger": trigger,
                    "invalid_pieces": invalid.total,
                    "sample_pieces": sample,
                    "affected_files": files,
                    "note": "repair rewrites the existing files in place, so hardlinks to them are healed; copies made earlier are not",
                }),
            )
            .await;
        }
    }

    /// Downgrade `Valid` pieces that overlap a hole or unwritten extent of any
    /// file, so they are re-hashed instead of trusted. Returns the pieces
    /// downgraded and the files whose filesystem could not report allocation.
    async fn audit_allocation(&mut self, state: &mut FastresumeState) -> (u32, u64) {
        let files: Vec<(u32, PathBuf, u64)> = self
            .meta
            .files
            .iter()
            .filter(|file| !file.pad && file.length > 0)
            .map(|file| (file.index, file.path.resolve(&self.save_root), file.length))
            .collect();
        if files.is_empty() {
            return (0, 0);
        }
        let piece_map = Arc::clone(&self.piece_map);
        let audited =
            tokio::task::spawn_blocking(move || audit_files_blocking(&piece_map, files)).await;
        match audited {
            Ok((ranges, unsupported)) => (state.distrust_piece_ranges(&ranges), unsupported),
            Err(error) => {
                warn!(
                    component = "fastresume",
                    operation = "allocation_audit",
                    torrent = %self.info_hash_hex,
                    error = %error,
                    "allocation audit task failed; continuing without it"
                );
                (0, 0)
            }
        }
    }

    async fn save_fastresume(&mut self, full_verify: bool) -> SaveOutcome {
        self.save_fastresume_with(full_verify, false).await
    }

    /// The final save before this task exits (orderly shutdown, demotion to
    /// dormant). It always runs a real payload barrier, even in `fast`
    /// durability mode: the run marker will call a clean exit "clean", and a
    /// clean exit must mean the data is on disk, not merely in the page cache
    /// of a machine that might lose power a moment later.
    async fn save_fastresume_for_exit(&mut self) -> SaveOutcome {
        self.save_fastresume_with(false, true).await
    }

    /// Write the fastresume record. With `force_sync`, the payload fsync
    /// barrier runs even in `fast` durability mode (the completion gate needs
    /// a real barrier regardless of mode).
    async fn save_fastresume_with(&mut self, full_verify: bool, force_sync: bool) -> SaveOutcome {
        let durable_assembly_pieces = self.flush_piece_assemblies_to_disk().await;
        let (uploaded, downloaded) = self.transfer_snapshot().await;
        let mut state = FastresumeState::new_empty(
            &self.meta.info_hash,
            self.meta.pieces.len() as u32,
            ImportPolicy::RequireVerification,
        );
        for (piece, piece_state) in state.pieces.iter_mut().enumerate() {
            *piece_state = if self.picker.have_piece(piece) {
                PieceState::Valid
            } else {
                PieceState::Unknown
            };
        }
        state.partial_pieces = self
            .picker
            .partial_pieces_limited(MAX_FASTRESUME_BLOCKS_PER_PARTIAL_PIECE)
            .into_iter()
            .filter(|(piece, _)| {
                !self.pending_transfer_pieces.get(piece).is_some_and(|work| {
                    work.verify_ready || work.verify_in_flight || work.aggregate_write_ready
                }) && (!self.piece_assemblies.contains_key(piece)
                    || durable_assembly_pieces.contains(piece))
            })
            .map(|(piece, received_blocks)| PartialPieceState {
                piece,
                received_blocks,
            })
            .collect();
        state.uploaded_bytes = uploaded;
        state.downloaded_bytes = downloaded;
        if self.dirty_pieces_since_barrier.is_overflowed() {
            // An empty watermark with clean_shutdown=false is intentional:
            // the loader treats it as unverifiable and performs a full
            // recheck instead of trusting an incomplete dirty-piece list.
            warn!(
                component = "fastresume",
                operation = "save",
                torrent = %self.info_hash_hex,
                result = "watermark_overflow",
                maximum = MAX_RUNTIME_DIRTY_PIECES,
                "dirty-piece watermark exceeded its live bound; an unclean save will require a full recheck"
            );
        } else {
            state.set_dirty_pieces_since_barrier(
                self.dirty_pieces_since_barrier.pieces.iter().copied(),
            );
        }
        let (mut barrier, evidence) = self.run_barrier(force_sync).await;
        let mut evidence = if barrier == BarrierOutcome::Synced {
            evidence
        } else {
            drop(evidence);
            None
        };
        match barrier {
            BarrierOutcome::Synced if evidence.is_some() => {
                self.dirty_pieces_since_barrier.clear();
            }
            BarrierOutcome::Synced => {
                // Never let an impossible missing proof turn a successful
                // status into trusted fast-resume state.
                state.mark_saved_without_data_sync();
                state.clean_shutdown = false;
                barrier = BarrierOutcome::Failed;
            }
            // No data fsync happened (by configuration, or because this
            // filesystem cannot). The state is still good after a clean
            // shutdown or a process crash, but the loader must not trust it
            // after a host crash.
            BarrierOutcome::SkippedFast | BarrierOutcome::Unsupported => {
                state.complete_without_data_sync();
                self.dirty_pieces_since_barrier.clear();
            }
            BarrierOutcome::Failed => {
                state.clean_shutdown = false;
            }
        }
        state.durability.boot_id = self.crash_safety.boot_id().map(str::to_owned);
        state.durability.saved_by_run_id = self.crash_safety.run_id();
        state.durability.last_data_write_unix = self
            .last_data_write_unix
            .load(std::sync::atomic::Ordering::Relaxed);
        state.file_hints = collect_file_hints(&self.save_root, &self.meta);
        if full_verify {
            state.last_full_verify = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }

        let save_result = match evidence.take() {
            Some(evidence) => {
                self.fastresume
                    .save_async_with_durability_evidence(state, evidence)
                    .await
            }
            None => self.fastresume.save_async(state).await,
        };
        let persisted = match save_result {
            Ok(()) => true,
            Err(e) => {
                warn!(
                    component = "fastresume",
                    operation = "save",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %e,
                    "failed to save fastresume state"
                );
                false
            }
        };
        SaveOutcome { barrier, persisted }
    }

    /// Flush payload data before a fastresume record may claim it.
    ///
    /// `fast` mode skips this unless `force` (the completion gate) demands it.
    /// A filesystem that cannot fsync at all is detected once, remembered, and
    /// reported as `Unsupported` instead of failing every save forever.
    async fn run_barrier(
        &mut self,
        force: bool,
    ) -> (
        BarrierOutcome,
        Option<rt_storage::DurabilityBarrierEvidence>,
    ) {
        if self.fsync_unsupported {
            return (BarrierOutcome::Unsupported, None);
        }
        if !force && self.storage.io_config().durability_mode == rt_storage::DurabilityMode::Fast {
            return (BarrierOutcome::SkippedFast, None);
        }
        match self.storage.sync_all_open_files_with_evidence().await {
            Ok(evidence) => (BarrierOutcome::Synced, Some(evidence)),
            Err(e) if e.is_operation_unsupported() => {
                self.fsync_unsupported = true;
                self.crash_safety.note_fsync_unsupported(&self.save_root);
                warn!(
                    component = "storage",
                    operation = "sync_before_fastresume",
                    torrent = %self.info_hash_hex,
                    result = "unsupported",
                    error = %e,
                    "this filesystem cannot fsync; durability is best effort and the mount is treated as weak"
                );
                (BarrierOutcome::Unsupported, None)
            }
            Err(e) => {
                warn!(
                    component = "storage",
                    operation = "sync_before_fastresume",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %e,
                    "failed to sync torrent files before fastresume save"
                );
                (BarrierOutcome::Failed, None)
            }
        }
    }

    /// Make partial in-memory assemblies durable before advertising their
    /// received block indexes in fastresume. A clean fastresume record that
    /// points at bytes which only existed in RAM would make the next process
    /// trust a partial piece that the disk does not contain.
    async fn flush_piece_assemblies_to_disk(&mut self) -> HashSet<u32> {
        let mut durable = HashSet::new();
        let pieces = self.piece_assemblies.keys().copied().collect::<Vec<_>>();
        for piece in pieces {
            let blocks = match self
                .piece_assemblies
                .get(&piece)
                .ok_or_else(|| anyhow::anyhow!("piece assembly disappeared"))
                .and_then(|assembly| assembly.received_blocks(&self.resources, piece))
            {
                Ok(blocks) => blocks,
                Err(error) => {
                    warn!(
                        component = "fastresume",
                        operation = "flush_piece_assembly",
                        torrent = %self.info_hash_hex,
                        piece,
                        result = "error",
                        error = %error,
                        "could not reserve memory for partial-piece flush"
                    );
                    continue;
                }
            };
            let mut success = true;
            for (offset, data) in blocks {
                if let Err(error) = self.write_leased_block(&data).await {
                    warn!(
                        component = "fastresume",
                        operation = "flush_piece_assembly",
                        torrent = %self.info_hash_hex,
                        piece,
                        offset,
                        result = "error",
                        error = %error,
                        "could not flush an in-memory partial piece before fastresume save"
                    );
                    success = false;
                    break;
                }
            }
            if success {
                durable.insert(piece);
            }
        }
        durable
    }
}

/// Resolve at `deadline`; callers guard on the deadline being set.
async fn sleep_until_download_tokens(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(TokioInstant::from_std(deadline)).await;
    }
}

fn reset_webseed_sleep(sleep: &mut Pin<Box<Sleep>>, delay: Duration) {
    sleep.as_mut().reset(tokio::time::Instant::now() + delay);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutgoingTransportPolicy {
    Auto,
    TcpOnly,
    PreferUtp,
    UtpOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerSource {
    Tracker,
    Dht,
    PeerExchange,
    Manual,
}

pub(crate) fn outgoing_transport_policy_configured() -> OutgoingTransportPolicy {
    if let Ok(value) = std::env::var("TNG_UTP_OUTGOING") {
        return parse_outgoing_transport_policy(&value);
    }
    if std::env::var_os("TNG_ENABLE_UTP_OUTGOING").is_some() {
        return OutgoingTransportPolicy::PreferUtp;
    }
    OutgoingTransportPolicy::Auto
}

fn parse_outgoing_transport_policy(value: &str) -> OutgoingTransportPolicy {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "default" => OutgoingTransportPolicy::Auto,
        "0" | "false" | "no" | "off" | "tcp" | "tcp-only" => OutgoingTransportPolicy::TcpOnly,
        "1" | "true" | "yes" | "prefer" | "prefer-utp" | "utp-prefer" => {
            OutgoingTransportPolicy::PreferUtp
        }
        "only" | "utp" | "utp-only" => OutgoingTransportPolicy::UtpOnly,
        _ => OutgoingTransportPolicy::Auto,
    }
}

pub(crate) fn outgoing_transport_policy_for_peer(
    configured: OutgoingTransportPolicy,
    source: PeerSource,
    private: bool,
) -> OutgoingTransportPolicy {
    if private {
        return OutgoingTransportPolicy::TcpOnly;
    }
    match configured {
        OutgoingTransportPolicy::Auto => match source {
            PeerSource::Tracker => OutgoingTransportPolicy::TcpOnly,
            PeerSource::Dht | PeerSource::PeerExchange | PeerSource::Manual => {
                OutgoingTransportPolicy::PreferUtp
            }
        },
        explicit => explicit,
    }
}

/// Collect on-disk size/mtime/inode hints for every file in `meta`.
///
/// Per-file, not all-or-nothing: a file that can't be stat'd (missing,
/// permission error, a BEP47 padding file real clients never materialize,
/// ...) is simply omitted from the result rather than aborting the whole
/// collection. `apply_file_hints` already treats a missing hint as "this
/// file changed" and invalidates only *that* file's pieces — one bad file
/// must not poison fastresume trust for every other file in the torrent.
fn collect_file_hints(root: &std::path::Path, meta: &TorrentMetaV1) -> Vec<FileHint> {
    meta.files
        .iter()
        .filter(|file| !file.pad)
        .filter_map(|file| {
            let path = file.path.resolve(root);
            let metadata = match rt_storage::metadata_no_follow(&path) {
                Ok(metadata) => metadata,
                Err(e) => {
                    debug!(
                        component = "fastresume",
                        operation = "collect_file_hints",
                        file_index = file.index,
                        path = %path.display(),
                        error = %e,
                        "could not stat file; omitting its hint"
                    );
                    return None;
                }
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);

            #[cfg(unix)]
            let inode = {
                use std::os::unix::fs::MetadataExt;
                metadata.ino()
            };
            #[cfg(not(unix))]
            let inode = 0;

            Some(FileHint {
                file_index: file.index,
                size: metadata.len(),
                mtime_secs: modified,
                inode,
            })
        })
        .collect()
}

fn tracker_tiers_from_meta(meta: &TorrentMetaV1) -> Vec<Vec<TrackerState>> {
    let mut seen = std::collections::HashSet::new();
    let mut tiers = Vec::new();

    if !meta.announce_list.is_empty() {
        for tier in &meta.announce_list {
            let trackers: Vec<TrackerState> = tier
                .iter()
                .filter_map(|url| {
                    if seen.insert(url.clone()) {
                        Some(TrackerState::new(url.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            if !trackers.is_empty() {
                tiers.push(trackers);
            }
        }
    }

    if let Some(url) = &meta.announce {
        if seen.insert(url.clone()) {
            tiers.insert(0, vec![TrackerState::new(url.clone())]);
        }
    }

    tiers
}

pub(crate) fn tracker_tiers_from_urls(urls: &[String]) -> Vec<Vec<TrackerState>> {
    let mut seen = HashSet::new();
    let trackers = urls
        .iter()
        .filter_map(|url| {
            if url.is_empty() || !seen.insert(url.clone()) {
                None
            } else {
                Some(TrackerState::new(url.clone()))
            }
        })
        .collect::<Vec<_>>();
    if trackers.is_empty() {
        Vec::new()
    } else {
        vec![trackers]
    }
}

/// Restore the small amount of tracker session state that must survive an
/// actor restart. Tracker IDs are opaque BEP 3 bytes and cannot be recovered
/// from the metainfo; losing them makes the next HTTP announce a new tracker
/// session. A persisted next deadline is restored as well so startup does
/// not create an announce storm for every resumed torrent. The query itself
/// is issued through `DbExecutor` before this pure projection runs.
pub(crate) fn restore_tracker_state_from_rows(
    tiers: &mut [Vec<TrackerState>],
    rows: &[rt_db::TorrentTrackerRow],
) {
    let persisted = rows
        .iter()
        .map(|row| (row.url.clone(), row))
        .collect::<HashMap<_, _>>();
    let now = Instant::now();
    let now_unix = db_i64(unix_now());
    for tier in tiers {
        for tracker in tier {
            let Some(row) = persisted.get(&tracker.url) else {
                continue;
            };
            tracker.tracker_id = row.tracker_id.clone();
            tracker.next_announce = row.next_announce_at.map(|deadline| {
                let delay = deadline.saturating_sub(now_unix).max(0) as u64;
                now + Duration::from_secs(delay)
            });
        }
    }
}

pub(crate) fn private_peer_source_allowed(
    is_private: bool,
    allowed_private_peers: &HashSet<SocketAddr>,
    peer: SocketAddr,
) -> bool {
    !is_private
        || allowed_private_peers.contains(&peer)
        || (private_peer_port_fallback_allowed(peer.ip())
            && allowed_private_peers
                .iter()
                .any(|allowed| allowed.ip() == peer.ip()))
}

pub(crate) fn private_peer_port_fallback_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                // PERF-110 uses RFC 2544's reserved benchmarking block for
                // isolated peers. It is non-routable like RFC 1918 for the
                // same-IP source-port fallback used by private torrents.
                || (octets[0] == 198 && matches!(octets[1], 18 | 19))
        }
        IpAddr::V6(ip) => {
            ip.is_loopback() || is_ipv6_unique_local(ip) || is_ipv6_unicast_link_local(ip)
        }
    }
}

fn is_ipv6_unique_local(ip: std::net::Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

fn is_ipv6_unicast_link_local(ip: std::net::Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}

fn record_peer_transfer(peer: &mut PeerHandle, upload: bool, bytes: u64) {
    let now = Instant::now();
    if upload {
        peer.uploaded = peer.uploaded.saturating_add(bytes);
        peer.upload_rate_window = peer.upload_rate_window.saturating_add(bytes);
        let elapsed = now.saturating_duration_since(peer.upload_rate_window_started);
        if elapsed < Duration::from_secs(1) {
            return;
        }
        peer.upload_rate = peer.upload_rate_window as f64 / elapsed.as_secs_f64().max(0.001);
        peer.upload_rate_window = 0;
        peer.upload_rate_window_started = now;
    } else {
        peer.downloaded = peer.downloaded.saturating_add(bytes);
        peer.download_rate_window = peer.download_rate_window.saturating_add(bytes);
        let elapsed = now.saturating_duration_since(peer.download_rate_window_started);
        if elapsed < Duration::from_secs(1) {
            return;
        }
        peer.download_rate = peer.download_rate_window as f64 / elapsed.as_secs_f64().max(0.001);
        peer.download_rate_window = 0;
        peer.download_rate_window_started = now;
    }
}

fn peer_rate(rate: f64, last_sample: Instant) -> i64 {
    if Instant::now().saturating_duration_since(last_sample) > Duration::from_secs(15) {
        0
    } else {
        rate.max(0.0).round() as i64
    }
}

fn peer_client_label(peer: &PeerHandle) -> String {
    if peer.ut_metadata_id.is_some() {
        "BEP10 peer".to_owned()
    } else {
        "BitTorrent peer".to_owned()
    }
}

fn webseed_block_url(meta: &TorrentMetaV1, webseed: &str) -> Option<Url> {
    let parsed = Url::parse(webseed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let first_file = meta.files.first()?;
    let components = first_file.path.components();
    if !webseed.ends_with('/')
        && components.len() > 1
        && components
            .first()
            .is_some_and(|component| component == &meta.name)
        && parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .is_some_and(|last| last == meta.name)
    {
        let mut url = parsed;
        {
            let mut segments = url.path_segments_mut().ok()?;
            for component in &components[1..] {
                segments.push(component);
            }
        }
        return Some(url);
    }
    if webseed.ends_with('/') {
        parsed.join(&meta.name).ok()
    } else {
        Some(parsed)
    }
}

fn webseed_byte_range(
    piece: u32,
    piece_length: u64,
    begin: u32,
    length: u32,
) -> anyhow::Result<(u64, u64)> {
    if piece_length == 0 {
        anyhow::bail!("webseed piece length must not be zero");
    }
    if length == 0 || length > MAX_BLOCK_SIZE {
        anyhow::bail!("webseed block length {length} is invalid");
    }
    let piece_start = u64::from(piece)
        .checked_mul(piece_length)
        .ok_or_else(|| anyhow::anyhow!("webseed piece offset overflow"))?;
    let start = piece_start
        .checked_add(u64::from(begin))
        .ok_or_else(|| anyhow::anyhow!("webseed block start overflow"))?;
    let end = start
        .checked_add(u64::from(length - 1))
        .ok_or_else(|| anyhow::anyhow!("webseed block end overflow"))?;
    Ok((start, end))
}

fn validate_webseed_range_response(
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    expected_start: u64,
    expected_end: u64,
) -> anyhow::Result<()> {
    if status != StatusCode::PARTIAL_CONTENT {
        // A 200 response may contain the complete file from byte zero even
        // though the server ignored our Range header. Accepting and
        // truncating that body would write the wrong bytes for every
        // non-zero block.
        anyhow::bail!("webseed did not honor byte range: HTTP {status}");
    }
    let content_range = headers
        .get(CONTENT_RANGE)
        .ok_or_else(|| anyhow::anyhow!("webseed 206 response is missing Content-Range"))?
        .to_str()
        .map_err(|_| anyhow::anyhow!("webseed Content-Range is not valid ASCII"))?;
    let range = content_range
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .and_then(|(range, _total)| range.split_once('-'));
    let Some((start, end)) = range else {
        anyhow::bail!("webseed returned malformed Content-Range: {content_range}");
    };
    let start = start.parse::<u64>().map_err(|_| {
        anyhow::anyhow!("webseed returned malformed Content-Range: {content_range}")
    })?;
    let end = end.parse::<u64>().map_err(|_| {
        anyhow::anyhow!("webseed returned malformed Content-Range: {content_range}")
    })?;
    if start != expected_start || end != expected_end {
        anyhow::bail!(
            "webseed returned byte range {start}-{end}, expected {expected_start}-{expected_end}"
        );
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct UtPexPeers {
    added: Vec<SocketAddr>,
    dropped: Vec<SocketAddr>,
}

/// Parses BEP 11 compact IPv4/IPv6 `added`, `added6`, `dropped`, and
/// `dropped6` lists. The dropped set remains advisory at the task boundary:
/// it removes stale retry candidates but never forcibly disconnects an
/// otherwise healthy local connection.
fn parse_ut_pex_peers(payload: &[u8]) -> anyhow::Result<UtPexPeers> {
    let value = Decoder::new(payload)
        .with_max_nodes(MAX_UT_PEX_BENCODE_NODES)
        .decode()?;
    let BValue::Dict(pairs) = value else {
        anyhow::bail!("ut_pex payload must be a dict");
    };
    let parse_field = |key: &[u8], width: usize| -> anyhow::Result<Vec<SocketAddr>> {
        let Some(bytes) = pairs
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .and_then(|(_, value)| value.as_bytes())
        else {
            return Ok(Vec::new());
        };
        if bytes.len() % width != 0 {
            anyhow::bail!("ut_pex {key:?} peers length is not a multiple of {width}");
        }
        let count = bytes.len() / width;
        if count > MAX_UT_PEX_PEERS {
            anyhow::bail!("ut_pex {key:?} peer list exceeds {MAX_UT_PEX_PEERS} entries");
        }
        let mut peers = Vec::with_capacity(count);
        if width == 6 {
            for chunk in bytes.as_chunks::<6>().0 {
                let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                if port != 0 {
                    peers.push(SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]),
                        port,
                    )));
                }
            }
        } else {
            for chunk in bytes.as_chunks::<18>().0 {
                let port = u16::from_be_bytes([chunk[16], chunk[17]]);
                if port != 0 {
                    let octets: [u8; 16] = chunk[0..16].try_into().expect("chunk is 18 bytes");
                    peers.push(SocketAddr::V6(SocketAddrV6::new(
                        Ipv6Addr::from(octets),
                        port,
                        0,
                        0,
                    )));
                }
            }
        }
        Ok(peers)
    };
    let added = parse_field(b"added", 6)?;
    let added6 = parse_field(b"added6", 18)?;
    let dropped = parse_field(b"dropped", 6)?;
    let dropped6 = parse_field(b"dropped6", 18)?;
    if added.len() + added6.len() + dropped.len() + dropped6.len() > MAX_UT_PEX_PEERS {
        anyhow::bail!("ut_pex peer list exceeds {MAX_UT_PEX_PEERS} entries");
    }
    Ok(UtPexPeers {
        added: added.into_iter().chain(added6).collect(),
        dropped: dropped.into_iter().chain(dropped6).collect(),
    })
}

fn reconcile_peer_availability<A: PieceAvailability + ?Sized, B: PieceAvailability + ?Sized>(
    availability: &mut Availability,
    old: &A,
    new: &B,
) {
    let piece_count = availability.piece_count();
    for piece in 0..piece_count {
        match (old.has_piece(piece), new.has_piece(piece)) {
            (false, true) => availability.add_have(piece),
            (true, false) => availability.remove_have(piece),
            _ => {}
        }
    }
}

fn remove_peer_availability(availability: &mut Availability, peer_has: &PieceBitmap) {
    for piece in 0..availability.piece_count() {
        if peer_has.get(piece).unwrap_or(false) {
            availability.remove_have(piece);
        }
    }
}

fn tracker_event_after_success(current: TrackerEvent, sent: TrackerEvent) -> TrackerEvent {
    // A newer lifecycle event may have been queued while this request was in
    // flight (for example, the torrent can finish while its Started announce
    // is waiting on the tracker). Only consume the event represented by the
    // response; otherwise a successful Started response can erase a pending
    // Completed notification.
    if current == sent && matches!(sent, TrackerEvent::Started | TrackerEvent::Completed) {
        TrackerEvent::Empty
    } else {
        current
    }
}

fn tracker_event_after_incomplete_recheck(current: TrackerEvent) -> TrackerEvent {
    // A Started event may still be pending for a newly resumed session. A
    // Completed or Stopped event, however, no longer describes an active
    // incomplete torrent after the recheck and must not be sent later.
    match current {
        TrackerEvent::Started => TrackerEvent::Started,
        TrackerEvent::Empty | TrackerEvent::Completed | TrackerEvent::Stopped => {
            TrackerEvent::Empty
        }
    }
}

fn consume_stopped_announce(stopped_announced: &mut bool) -> bool {
    if *stopped_announced {
        false
    } else {
        *stopped_announced = true;
        true
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn instant_to_unix(instant: Option<Instant>, now: Instant) -> Option<i64> {
    let current = db_i64(unix_now());
    instant.map(|instant| {
        if instant >= now {
            current.saturating_add(db_i64(instant.duration_since(now).as_secs()))
        } else {
            current.saturating_sub(db_i64(now.duration_since(instant).as_secs()))
        }
    })
}

fn tracker_status_label(status: &TrackerStatus) -> &'static str {
    match status {
        TrackerStatus::NeverAnnounced => "never_announced",
        TrackerStatus::Announcing => "announcing",
        TrackerStatus::Working => "working",
        TrackerStatus::Warning(_) => "warning",
        TrackerStatus::Error(_) => "error",
        TrackerStatus::Disabled => "disabled",
    }
}

fn tracker_failure_reason(status: &TrackerStatus) -> Option<String> {
    match status {
        TrackerStatus::Error(err) => Some(err.to_string()),
        _ => None,
    }
}

fn tracker_warning_message(status: &TrackerStatus) -> Option<String> {
    match status {
        TrackerStatus::Warning(message) => Some(message.clone()),
        _ => None,
    }
}

fn peer_event_if_active(paused: bool, event: PeerEvent) -> Option<PeerEvent> {
    (!paused).then_some(event)
}

async fn send_peer_event(peer_event_tx: &mpsc::Sender<PeerEvent>, event: PeerEvent) -> bool {
    matches!(
        timeout(PEER_EVENT_SEND_TIMEOUT, peer_event_tx.send(event)).await,
        Ok(Ok(()))
    )
}

async fn send_choke_event(
    peer_event_tx: &mpsc::Sender<PeerEvent>,
    peer: SocketAddr,
    id: PeerId,
    download_generation: u64,
    fast_negotiated: bool,
    choked_requests: Vec<BlockRequest>,
    outstanding: &mut Vec<OutstandingRequest>,
) -> bool {
    if send_peer_event(
        peer_event_tx,
        PeerEvent::Choked {
            peer,
            id,
            download_generation,
            fast_negotiated,
            outstanding: choked_requests.clone(),
        },
    )
    .await
    {
        true
    } else {
        // The actor still owns these reservations because the Choked event
        // was not delivered. Keep them for the terminal disconnect fallback.
        if !fast_negotiated {
            outstanding.extend(
                choked_requests
                    .into_iter()
                    .map(|request| OutstandingRequest::new(request, None)),
            );
        }
        false
    }
}

/// Terminal peer events must eventually reach the actor while its channel is
/// still open. Dropping a disconnect because the dedicated queue is full
/// strands the active peer entry, its picker reservations, and its global
/// peer permit. The peer task is owned by the torrent actor, so shutdown can
/// abort this wait if the actor itself is no longer making progress.
async fn send_peer_disconnect_event(
    peer_event_tx: &mpsc::Sender<PeerEvent>,
    event: PeerEvent,
) -> bool {
    peer_event_tx.send(event).await.is_ok()
}

/// Open a TCP connection, complete BEP 3 handshake, receive Piece messages.
async fn run_outgoing_peer_with_policy(
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
    policy: OutgoingTransportPolicy,
) -> anyhow::Result<PeerLoopExit> {
    match policy {
        OutgoingTransportPolicy::Auto => {
            run_outgoing_peer(addr, peer_id, info_hash, peer_event_tx, peer_cmd_rx, upload).await
        }
        OutgoingTransportPolicy::TcpOnly => {
            run_outgoing_peer(addr, peer_id, info_hash, peer_event_tx, peer_cmd_rx, upload).await
        }
        OutgoingTransportPolicy::UtpOnly => {
            run_outgoing_utp_peer(addr, peer_id, info_hash, peer_event_tx, peer_cmd_rx, upload)
                .await
        }
        OutgoingTransportPolicy::PreferUtp => {
            match connect_utp_with_deadline(&upload.network_budget, addr).await {
                Ok(stream) => {
                    match prepare_outgoing_utp_peer(addr, info_hash, &upload, stream).await {
                        Ok((peer_io, fast)) => {
                            upload.fast_negotiated = fast;
                            Ok(run_peer_loop(
                                addr,
                                peer_id,
                                peer_io,
                                peer_event_tx,
                                peer_cmd_rx,
                                upload,
                                false,
                            )
                            .await)
                        }
                        Err(e) => {
                            debug!(
                                component = "peer",
                                operation = "setup_utp",
                                peer = %addr,
                                result = "fallback",
                                error = %e,
                                "uTP peer setup failed; falling back to TCP"
                            );
                            run_outgoing_peer(
                                addr,
                                peer_id,
                                info_hash,
                                peer_event_tx,
                                peer_cmd_rx,
                                upload,
                            )
                            .await
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        component = "peer",
                        operation = "connect_utp",
                        peer = %addr,
                        result = "fallback",
                        error = %e,
                        "uTP peer path failed; falling back to TCP"
                    );
                    run_outgoing_peer(addr, peer_id, info_hash, peer_event_tx, peer_cmd_rx, upload)
                        .await
                }
            }
        }
    }
}

async fn run_outgoing_peer(
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
) -> anyhow::Result<PeerLoopExit> {
    let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;

    let mut framed = Framed::with_capacity(
        stream,
        PeerCodec::with_resources(upload.resources.clone()),
        1,
    );

    let our_hs = Handshake {
        info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_extension_protocol_and_fast(),
    };
    // Send our handshake as raw bytes before the codec takes over.
    {
        use tokio::io::AsyncWriteExt;
        let inner = framed.get_mut();
        timeout(PEER_SOCKET_WRITE_TIMEOUT, inner.write_all(&our_hs.encode()))
            .await
            .map_err(|_| anyhow::anyhow!("peer handshake write timed out"))??;
    }

    let (remote_supports_extension, remote_supports_fast) = {
        use tokio::io::AsyncReadExt;
        let mut hs_buf = [0u8; 68];
        tokio::time::timeout(
            Duration::from_secs(10),
            framed.get_mut().read_exact(&mut hs_buf),
        )
        .await??;
        let remote_hs = Handshake::parse(&hs_buf)?;
        if remote_hs.info_hash != info_hash {
            anyhow::bail!("info_hash mismatch from {addr}");
        }
        if remote_hs.peer_id == crate::peer_id::our_peer_id() {
            anyhow::bail!("peer handshake identifies this client");
        }
        (
            remote_hs.reserved.supports_extension_protocol(),
            remote_hs.reserved.supports_fast_extension(),
        )
    };

    let mut peer_io = PeerIo::Tcp(framed);
    send_extension_handshake(
        &mut peer_io,
        upload.metadata.as_ref(),
        upload.is_private,
        upload.pex_enabled,
        remote_supports_extension,
    )
    .await?;
    upload.fast_negotiated = remote_supports_fast;
    send_have_state(
        &mut peer_io,
        &upload.have_pieces,
        &upload.resources,
        remote_supports_fast,
    )
    .await?;
    peer_io.send(Message::Interested).await?;

    Ok(run_peer_loop(
        addr,
        peer_id,
        peer_io,
        peer_event_tx,
        peer_cmd_rx,
        upload,
        false,
    )
    .await)
}

async fn run_outgoing_utp_peer(
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    upload: UploadContext,
) -> anyhow::Result<PeerLoopExit> {
    let stream = connect_utp_with_deadline(&upload.network_budget, addr).await?;
    run_established_utp_peer(
        addr,
        peer_id,
        info_hash,
        peer_event_tx,
        peer_cmd_rx,
        upload,
        stream,
    )
    .await
}

async fn prepare_outgoing_utp_peer(
    addr: SocketAddr,
    info_hash: [u8; 20],
    upload: &UploadContext,
    mut stream: UtpStream,
) -> anyhow::Result<(PeerIo, bool)> {
    let our_hs = Handshake {
        info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_extension_protocol_and_fast(),
    };
    timeout(
        PEER_SOCKET_WRITE_TIMEOUT,
        stream.write_all(&our_hs.encode()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("peer handshake write timed out"))??;

    let mut hs_buf = [0u8; 68];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut hs_buf)).await??;
    let remote_hs = Handshake::parse(&hs_buf)?;
    if remote_hs.info_hash != info_hash {
        anyhow::bail!("info_hash mismatch from {addr}");
    }
    if remote_hs.peer_id == crate::peer_id::our_peer_id() {
        anyhow::bail!("peer handshake identifies this client");
    }
    let remote_supports_extension = remote_hs.reserved.supports_extension_protocol();
    let remote_supports_fast = remote_hs.reserved.supports_fast_extension();

    let mut peer_io = PeerIo::Utp(Box::new(UtpPeerIo::new(stream, upload.resources.clone())));
    send_extension_handshake(
        &mut peer_io,
        upload.metadata.as_ref(),
        upload.is_private,
        upload.pex_enabled,
        remote_supports_extension,
    )
    .await?;
    send_have_state(
        &mut peer_io,
        &upload.have_pieces,
        &upload.resources,
        remote_supports_fast,
    )
    .await?;
    peer_io.send(Message::Interested).await?;
    Ok((peer_io, remote_supports_fast))
}

async fn run_established_utp_peer(
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
    stream: UtpStream,
) -> anyhow::Result<PeerLoopExit> {
    let (peer_io, fast) = prepare_outgoing_utp_peer(addr, info_hash, &upload, stream).await?;
    upload.fast_negotiated = fast;

    Ok(run_peer_loop(
        addr,
        peer_id,
        peer_io,
        peer_event_tx,
        peer_cmd_rx,
        upload,
        false,
    )
    .await)
}

#[allow(clippy::too_many_arguments)]
async fn run_incoming_peer(
    stream: TcpStream,
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
    remote_supports_extension: bool,
    remote_supports_fast: bool,
) -> anyhow::Result<PeerLoopExit> {
    stream.set_nodelay(true)?;
    let mut framed = Framed::with_capacity(
        stream,
        PeerCodec::with_resources(upload.resources.clone()),
        1,
    );

    let our_hs = Handshake {
        info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_extension_protocol_and_fast(),
    };
    {
        use tokio::io::AsyncWriteExt;
        timeout(
            PEER_SOCKET_WRITE_TIMEOUT,
            framed.get_mut().write_all(&our_hs.encode()),
        )
        .await
        .map_err(|_| anyhow::anyhow!("peer handshake write timed out"))??;
    }

    let mut peer_io = PeerIo::Tcp(framed);
    send_extension_handshake(
        &mut peer_io,
        upload.metadata.as_ref(),
        upload.is_private,
        upload.pex_enabled,
        remote_supports_extension,
    )
    .await?;
    upload.fast_negotiated = remote_supports_fast;
    send_have_state(
        &mut peer_io,
        &upload.have_pieces,
        &upload.resources,
        remote_supports_fast,
    )
    .await?;
    peer_io.send(Message::Interested).await?;
    Ok(run_peer_loop(
        addr,
        peer_id,
        peer_io,
        peer_event_tx,
        peer_cmd_rx,
        upload,
        false,
    )
    .await)
}

#[allow(clippy::too_many_arguments)]
async fn run_incoming_utp_peer(
    mut stream: UtpStream,
    addr: SocketAddr,
    peer_id: PeerId,
    info_hash: [u8; 20],
    peer_event_tx: mpsc::Sender<PeerEvent>,
    peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
    remote_supports_extension: bool,
    remote_supports_fast: bool,
) -> anyhow::Result<PeerLoopExit> {
    let our_hs = Handshake {
        info_hash,
        peer_id: crate::peer_id::our_peer_id(),
        reserved: ExtensionFlags::with_extension_protocol_and_fast(),
    };
    timeout(
        PEER_SOCKET_WRITE_TIMEOUT,
        stream.write_all(&our_hs.encode()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("peer handshake write timed out"))??;

    let mut peer_io = PeerIo::Utp(Box::new(UtpPeerIo::new(stream, upload.resources.clone())));
    send_extension_handshake(
        &mut peer_io,
        upload.metadata.as_ref(),
        upload.is_private,
        upload.pex_enabled,
        remote_supports_extension,
    )
    .await?;
    upload.fast_negotiated = remote_supports_fast;
    send_have_state(
        &mut peer_io,
        &upload.have_pieces,
        &upload.resources,
        remote_supports_fast,
    )
    .await?;
    peer_io.send(Message::Interested).await?;
    Ok(run_peer_loop(
        addr,
        peer_id,
        peer_io,
        peer_event_tx,
        peer_cmd_rx,
        upload,
        false,
    )
    .await)
}

pub(crate) enum PeerIo {
    Tcp(Framed<TcpStream, PeerCodec>),
    Utp(Box<UtpPeerIo>),
}

pub(crate) struct UtpPeerIo {
    stream: UtpStream,
    decoder: UtpFrameDecoder,
    write_buffer: UtpWireBuffer,
}

impl UtpPeerIo {
    pub(crate) fn new(stream: UtpStream, resources: ResourceGovernor) -> Self {
        Self {
            stream,
            decoder: UtpFrameDecoder::new(resources),
            write_buffer: UtpWireBuffer::default(),
        }
    }
}

impl PeerIo {
    pub(crate) async fn send(&mut self, msg: Message) -> anyhow::Result<()> {
        timeout(PEER_SOCKET_WRITE_TIMEOUT, async {
            match self {
                PeerIo::Tcp(framed) => framed.send(msg).await.map_err(Into::into),
                PeerIo::Utp(io) => io.send(msg).await,
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("peer socket write timed out"))?
    }

    pub(crate) async fn next(&mut self) -> anyhow::Result<Option<Message>> {
        match self {
            PeerIo::Tcp(framed) => match framed.next().await {
                Some(result) => result.map(Some).map_err(Into::into),
                None => Ok(None),
            },
            PeerIo::Utp(io) => io.next().await,
        }
    }
}

impl UtpPeerIo {
    async fn send(&mut self, msg: Message) -> anyhow::Result<()> {
        let encoded = self.write_buffer.encode(&self.decoder, &msg)?;
        self.stream.write_all(encoded).await?;
        Ok(())
    }

    async fn next(&mut self) -> anyhow::Result<Option<Message>> {
        self.decoder.next_message(&mut self.stream).await
    }
}

/// Reusable uTP write storage. Unlike TCP's `Framed` write buffer, the uTP
/// stream path previously allocated a new `Vec<u8>` for every message. Keep
/// one bounded frame buffer per connection and retain its resource lease for
/// as long as the allocation is retained.
pub(crate) struct UtpWireBuffer {
    buffer: BytesMut,
    lease: Option<MemoryLease>,
}

impl Default for UtpWireBuffer {
    fn default() -> Self {
        Self {
            buffer: BytesMut::new(),
            lease: None,
        }
    }
}

impl UtpWireBuffer {
    pub(crate) fn encode<'a>(
        &'a mut self,
        decoder: &UtpFrameDecoder,
        msg: &Message,
    ) -> anyhow::Result<&'a [u8]> {
        let encoded_len = msg
            .encoded_len()
            .ok_or_else(|| anyhow::anyhow!("peer message exceeds the wire length limit"))?;
        self.ensure_capacity(decoder, encoded_len)?;
        self.buffer.clear();
        msg.encode_into(&mut self.buffer)?;
        Ok(self.buffer.as_ref())
    }

    fn ensure_capacity(
        &mut self,
        decoder: &UtpFrameDecoder,
        required: usize,
    ) -> anyhow::Result<()> {
        if self.buffer.capacity() < required {
            // Acquire the next frame allowance before replacing the existing
            // buffer. This preserves the admission check if a peer requests a
            // larger frame, while the old allocation remains reusable when
            // the new reservation is denied.
            let mut lease = decoder.reserve_outbound_buffer(required)?;
            let old_buffer = std::mem::replace(&mut self.buffer, BytesMut::new());
            let old_lease = self.lease.take();
            drop(old_buffer);
            drop(old_lease);

            let buffer = BytesMut::with_capacity(required);
            let capacity = u64::try_from(buffer.capacity())
                .map_err(|_| anyhow::anyhow!("uTP peer write buffer capacity overflow"))?;
            if capacity > lease.bytes() && !lease.try_grow(capacity - lease.bytes()) {
                anyhow::bail!("uTP peer write buffer allocation denied");
            }
            self.buffer = buffer;
            self.lease = Some(lease);
        } else {
            let capacity = u64::try_from(self.buffer.capacity())
                .map_err(|_| anyhow::anyhow!("uTP peer write buffer capacity overflow"))?;
            match &mut self.lease {
                Some(lease) if lease.bytes() < capacity => {
                    if !lease.try_grow(capacity - lease.bytes()) {
                        anyhow::bail!("uTP peer write buffer allocation denied");
                    }
                }
                None => {
                    self.lease = Some(decoder.reserve_outbound_buffer(self.buffer.capacity())?);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Incremental length-prefix decoding for uTP's byte stream. Unlike
/// `read_exact` into a local buffer, this keeps a partial frame across a
/// cancelled `next()` future. The peer loop intentionally cancels that future
/// whenever a command, upload completion, or timer wins its `select!`.
pub(crate) struct UtpFrameDecoder {
    buffer: Vec<u8>,
    resources: ResourceGovernor,
    // The decoder retains its backing allocation across frame boundaries so
    // it can preserve a partial frame when `next()` is cancelled. Keep a
    // lease for that capacity, plus one same-sized allowance for the parsed
    // message copy made by `Message::parse` while the wire frame is handled.
    memory_leases: Vec<MemoryLease>,
    reserved_bytes: u64,
}

impl Default for UtpFrameDecoder {
    fn default() -> Self {
        Self::new(ResourceGovernor::new(Default::default()))
    }
}

impl UtpFrameDecoder {
    pub(crate) fn new(resources: ResourceGovernor) -> Self {
        Self {
            buffer: Vec::new(),
            resources,
            memory_leases: Vec::new(),
            reserved_bytes: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub(crate) fn reserve_outbound_buffer(&self, bytes: usize) -> anyhow::Result<MemoryLease> {
        let bytes = u64::try_from(bytes)
            .map_err(|_| anyhow::anyhow!("peer message memory estimate does not fit in u64"))?;
        self.resources
            .try_acquire(MemoryClass::PeerBuffer, bytes)
            .ok_or_else(|| anyhow::anyhow!("peer message allocation of {bytes} bytes denied"))
    }

    pub(crate) async fn next_message(
        &mut self,
        stream: &mut UtpStream,
    ) -> anyhow::Result<Option<Message>> {
        loop {
            if let Some(message) = self.take_frame()? {
                return Ok(Some(message));
            }
            let chunk = match stream.recv().await {
                Ok(chunk) => chunk,
                Err(rt_utp::UtpError::Closed) => {
                    if self.is_empty() {
                        return Ok(None);
                    }
                    anyhow::bail!("uTP peer closed in the middle of a message")
                }
                Err(err) => return Err(err.into()),
            };
            if chunk.is_empty() {
                if self.is_empty() {
                    return Ok(None);
                }
                anyhow::bail!("uTP peer closed in the middle of a message");
            }
            self.push(&chunk)?;
        }
    }

    fn push(&mut self, chunk: &[u8]) -> anyhow::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }

        // Read only the length prefix before appending the rest of an input
        // chunk. A declared 2 MiB frame must be admitted before its backing
        // allocation grows; otherwise every uTP peer could retain one legal
        // maximum-sized partial frame outside the PeerBuffer governor.
        if self.buffer.len() < 4 {
            let header_len = (4 - self.buffer.len()).min(chunk.len());
            self.reserve_capacity(self.buffer.len().saturating_add(header_len))?;
            self.buffer.extend_from_slice(&chunk[..header_len]);
            if self.buffer.len() == 4 {
                self.reserve_declared_frame()?;
            }
            if header_len < chunk.len() {
                self.append_chunk(&chunk[header_len..])?;
            }
            return Ok(());
        }

        self.reserve_declared_frame()?;
        self.append_chunk(chunk)?;
        Ok(())
    }

    fn take_frame(&mut self) -> anyhow::Result<Option<Message>> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }
        self.reserve_declared_frame()?;
        let len = self.frame_length();
        let frame_len = 4usize
            .checked_add(len as usize)
            .expect("bounded peer frame length must fit usize");
        if self.buffer.len() < frame_len {
            return Ok(None);
        }
        // Parse directly from the retained frame before draining it. The
        // message parser already copies the payload variants that must outlive
        // this buffer (Piece, Bitfield, and Extended); materializing a second
        // full payload Vec here made the peak roughly three frame sizes while
        // the decoder only admitted two.
        let message = Message::parse(&self.buffer[4..frame_len])?;
        self.buffer.drain(..frame_len);
        Ok(Some(message))
    }

    fn append_chunk(&mut self, chunk: &[u8]) -> anyhow::Result<()> {
        let required = self
            .buffer
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("peer frame buffer length overflow"))?;
        self.reserve_capacity(required)?;
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    fn reserve_declared_frame(&mut self) -> anyhow::Result<()> {
        let len = self.frame_length();
        if len > rt_peer_wire::message::MAX_MESSAGE_LEN {
            anyhow::bail!("peer message too large: {len}");
        }
        let frame_len = 4usize
            .checked_add(len as usize)
            .expect("bounded peer frame length must fit usize");
        self.reserve_capacity(frame_len)
    }

    fn reserve_capacity(&mut self, required: usize) -> anyhow::Result<()> {
        if required <= self.buffer.capacity() {
            return Ok(());
        }
        let required_bytes = u64::try_from(required)
            .map_err(|_| anyhow::anyhow!("peer frame buffer length does not fit in u64"))?
            .saturating_mul(2);
        if required_bytes <= self.reserved_bytes {
            self.buffer
                .try_reserve_exact(required - self.buffer.capacity())
                .map_err(|error| anyhow::anyhow!("peer frame buffer allocation failed: {error}"))?;
            return Ok(());
        }
        let additional = required_bytes.saturating_sub(self.reserved_bytes);
        let lease = self
            .resources
            .try_acquire(MemoryClass::PeerBuffer, additional)
            .ok_or_else(|| {
                anyhow::anyhow!("peer frame buffer allocation of {required_bytes} bytes denied")
            })?;
        self.buffer
            .try_reserve_exact(required - self.buffer.capacity())
            .map_err(|error| anyhow::anyhow!("peer frame buffer allocation failed: {error}"))?;
        self.memory_leases.push(lease);
        self.reserved_bytes = required_bytes;
        Ok(())
    }

    fn frame_length(&self) -> u32 {
        let header = [
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ];
        u32::from_be_bytes(header)
    }
}

#[derive(Debug)]
struct PeerLoopExit {
    result: anyhow::Result<()>,
    /// Requests that were accepted by the torrent actor but had not yet
    /// produced a Piece event when the peer loop ended. The production
    /// wrapper forwards these to the actor's dedicated terminal channel so
    /// picker reservations are released even on a socket/error path.
    outstanding: Vec<BlockRequest>,
}

async fn run_peer_loop(
    addr: SocketAddr,
    peer_id: PeerId,
    mut peer_io: PeerIo,
    peer_event_tx: mpsc::Sender<PeerEvent>,
    mut peer_cmd_rx: mpsc::Receiver<PeerCommand>,
    mut upload: UploadContext,
    // Production wrappers use the dedicated terminal channel after this
    // loop returns. The direct loop path can opt into the ordinary event
    // channel when it owns no separate terminal delivery path.
    send_terminal_event_on_peer_channel: bool,
) -> PeerLoopExit {
    let mut outstanding = Vec::<OutstandingRequest>::new();
    // BEP 10 extension ids are negotiated per peer. The id in our outgoing
    // handshake is the id this loop receives from the peer; responses must
    // use the peer's advertised id rather than our local id.
    let mut remote_ut_metadata_id = None;
    let mut download_choked = true;
    let mut upload_choked = true;
    let mut timeout_tick = interval(Duration::from_secs(5));
    let mut last_activity = Instant::now();
    // Disk reads are scheduled through the bounded mount scheduler, but the
    // peer loop must not await one read inline: a slow device otherwise
    // prevents it from processing choke/shutdown/input messages. The loop
    // owns the socket; detached read futures only return a leased block.
    let mut upload_reads = UploadReadTasks::default();
    // The shared obligation owner accounts for queued and in-service requests
    // as one bounded per-connection state machine. Detached reads remain
    // capped independently and are refilled as completions arrive.
    let mut upload_obligations = InboundObligations::new(upload.fast_negotiated, MAX_UPLOAD_WORK);
    let mut upload_request_drops = 0u64;
    let mut download_generation = 0u64;

    let result: anyhow::Result<()> = async {
        loop {
        tokio::select! {
            cmd = peer_cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    // A dropped command sender means the owner can no
                    // longer control this peer. Do not leave the socket and
                    // its global peer permit alive until the idle timeout.
                    break;
                };
                last_activity = Instant::now();
                match cmd {
                    PeerCommand::Request {
                        request: req,
                        download_generation: request_generation,
                        memory_lease,
                    } => {
                        if download_choked || request_generation != download_generation {
                            // The engine may have queued a request just
                            // before the remote choke event reached it. Do
                            // not put that request on the wire; the engine's
                            // Choked event releases its reservation.
                            continue;
                        }
                        if let Err(error) = peer_io
                            .send(Message::Request {
                            piece: req.piece,
                            begin: req.begin,
                            length: req.length,
                        })
                        .await
                        {
                            // The torrent actor reserved this block before
                            // enqueueing the command. Preserve it for the
                            // terminal disconnect event when the socket
                            // rejects the write, otherwise the picker can
                            // strand the block as permanently requested.
                            outstanding.push(OutstandingRequest::new(req, Some(memory_lease)));
                            return Err(error);
                        }
                        outstanding.push(OutstandingRequest::new(req, Some(memory_lease)));
                    }
                    PeerCommand::Cancel { request } => {
                        if outstanding.iter().any(|outstanding| {
                            outstanding.req.piece == request.piece
                                && outstanding.req.begin == request.begin
                                && outstanding.req.length == request.length
                        }) {
                            peer_io
                                .send(Message::Cancel {
                                    piece: request.piece,
                                    begin: request.begin,
                                    length: request.length,
                                })
                                .await?;
                        }
                    }
                    PeerCommand::Have(piece) => {
                        upload.have_pieces.set(piece as usize, true);
                        peer_io.send(Message::Have(piece)).await?;
                    }
                    PeerCommand::Choke => {
                        upload.upload_control.cancel();
                        upload_choked = true;
                        peer_io.send(Message::Choke).await?;
                        for reject in upload_obligations.on_local_choke() {
                            send_upload_obligation_reject(&mut peer_io, reject).await?;
                        }
                        start_upload_reads(
                            &mut upload_reads,
                            &mut upload_obligations,
                            &upload,
                        );
                        if !upload.fast_negotiated {
                            // A local choke invalidates reads that were
                            // admitted under the previous upload state. Abort
                            // them now so a slow disk cannot retain peer
                            // memory until the per-read timeout.
                            abort_upload_reads(&upload_reads);
                        }
                    }
                    PeerCommand::Unchoke => {
                        upload.upload_control.cancel();
                        upload_choked = false;
                        upload_obligations.on_local_unchoke();
                        peer_io.send(Message::Unchoke).await?;
                        start_upload_reads(
                            &mut upload_reads,
                            &mut upload_obligations,
                            &upload,
                        );
                    }
                    PeerCommand::Shutdown => {
                        upload.upload_control.cancel();
                        upload.shutdown_control.cancel();
                        upload_obligations.close();
                        break;
                    }
                }
            }
            msg_result = peer_io.next() => {
                let Some(msg) = msg_result? else {
                    break;
                };
                last_activity = Instant::now();
                match msg {
                    Message::Bitfield(bits) => {
                        if let Err(error) = validate_bitfield_shape(&bits, upload.have_pieces.len()) {
                            debug!(
                                component = "peer",
                                operation = "parse_bitfield",
                                peer = %addr,
                                result = "error",
                                error = %error,
                                "ignoring invalid bitfield"
                            );
                            continue;
                        }
                        // The previous upload-side bitmap remains live until
                        // the actor replaces it, so reserve the incoming map
                        // before allocating it and account for both copies
                        // during this handoff.
                        let memory_lease = match reserve_peer_bitfield_len(
                            &upload.resources,
                            upload.have_pieces.len(),
                        ) {
                            Ok(lease) => lease,
                            Err(error) => {
                                debug!(
                                    component = "peer",
                                    operation = "reserve_bitfield_memory",
                                    peer = %addr,
                                    result = "denied",
                                    error = %error,
                                    "disconnecting peer because its bitfield exceeds the memory budget"
                                );
                                break;
                            }
                        };
                        let pieces = PieceBitmap::from_validated_bitfield(
                            &bits,
                            upload.have_pieces.len(),
                        );
                        // The packed bitmap is now the event's only copy. Do
                        // not retain the wire buffer while waiting for the
                        // bounded actor mailbox.
                        drop(bits);
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Bitfield {
                                peer: addr,
                                id: peer_id,
                                pieces,
                                _memory_lease: memory_lease,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::Have(piece) => {
                        if piece as usize >= upload.have_pieces.len() {
                            debug!(
                                component = "peer",
                                operation = "handle_have",
                                peer = %addr,
                                piece,
                                result = "ignored",
                                reason = "out_of_range",
                                "ignoring out-of-range have"
                            );
                            continue;
                        }
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Have {
                                peer: addr,
                                id: peer_id,
                                piece,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    // BEP 6 Fast extension: HaveAll/HaveNone stand in for a
                    // full or empty Bitfield. Feed the same availability
                    // path a real Bitfield would use so downstream piece
                    // selection/interest logic does not need to know the
                    // difference.
                    Message::HaveAll => {
                        let memory_lease = match reserve_peer_bitfield_len(
                            &upload.resources,
                            upload.have_pieces.len(),
                        ) {
                            Ok(lease) => lease,
                            Err(error) => {
                                debug!(
                                    component = "peer",
                                    operation = "reserve_bitfield_memory",
                                    peer = %addr,
                                    result = "denied",
                                    error = %error,
                                    "disconnecting peer because its fast-extension have-state exceeds the memory budget"
                                );
                                break;
                            }
                        };
                        let pieces = PieceBitmap::all_true(upload.have_pieces.len());
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Bitfield {
                                peer: addr,
                                id: peer_id,
                                pieces,
                                _memory_lease: memory_lease,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::HaveNone => {
                        let memory_lease = match reserve_peer_bitfield_len(
                            &upload.resources,
                            upload.have_pieces.len(),
                        ) {
                            Ok(lease) => lease,
                            Err(error) => {
                                debug!(
                                    component = "peer",
                                    operation = "reserve_bitfield_memory",
                                    peer = %addr,
                                    result = "denied",
                                    error = %error,
                                    "disconnecting peer because its fast-extension have-state exceeds the memory budget"
                                );
                                break;
                            }
                        };
                        let pieces = PieceBitmap::new(upload.have_pieces.len());
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Bitfield {
                                peer: addr,
                                id: peer_id,
                                pieces,
                                _memory_lease: memory_lease,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::Piece { piece, begin, data } => {
                        let data_len = data.len() as u32;
                        let Some(mut requested) =
                            take_matching_outstanding(&mut outstanding, piece, begin, data_len)
                        else {
                            warn!(
                                peer = %addr,
                                piece,
                                begin,
                                length = data_len,
                                "dropping unsolicited or mismatched piece block"
                            );
                            continue;
                        };
                        // Charge the aggregate budget for the payload that
                        // actually arrived, not for a request that might
                        // never have been fulfilled.
                        let control_generation = upload.shutdown_control.generation();
                        if !upload
                            .global_download
                            .acquire_or_cancelled(
                                data_len as u64,
                                &upload.shutdown_control,
                                control_generation,
                            )
                            .await
                            || upload.shutdown_control.generation() != control_generation
                        {
                            // Keep the original request lease with the
                            // outstanding coordinate while this response is
                            // discarded. It still covers the next response
                            // the peer may send for this request.
                            outstanding.push(requested);
                            continue;
                        }
                        let block = match LeasedBlockEvent::with_optional_lease(
                            BlockEvent {
                                piece,
                                offset: begin,
                                data,
                            },
                            requested.memory_lease.take(),
                        ) {
                            Ok(block) => block,
                            Err(error) => {
                                outstanding.push(OutstandingRequest::new(requested.req, None));
                                debug!(
                                    component = "peer",
                                    operation = "retain_piece_payload",
                                    peer = %addr,
                                    piece,
                                    begin,
                                    result = "rejected",
                                    error = %error,
                                    "disconnecting peer because received payload exceeded its reservation"
                                );
                                break;
                            }
                        };
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Piece {
                                peer: addr,
                                id: peer_id,
                                block,
                            },
                        )
                        .await
                        {
                            // The payload and its lease are dropped with the
                            // undelivered event. Keep only the coordinate so
                            // the terminal disconnect event releases the
                            // actor's picker reservation.
                            outstanding.push(OutstandingRequest::new(requested.req, None));
                            break; // torrent task gone
                        }
                    }
                    Message::Reject {
                        piece,
                        begin,
                        length,
                    } => {
                        let rejected = BlockRequest {
                            piece,
                            begin,
                            length,
                        };
                        if take_matching_outstanding(&mut outstanding, piece, begin, length)
                            .is_none()
                        {
                            debug!(
                                component = "peer",
                                operation = "handle_reject",
                                peer = %addr,
                                piece,
                                begin,
                                length,
                                result = "ignored",
                                reason = "unsolicited",
                                "ignoring reject for a request that is not outstanding"
                            );
                            continue;
                        }
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::RequestRejected {
                                peer: addr,
                                id: peer_id,
                                rejected,
                            },
                        )
                        .await
                        {
                            outstanding.push(OutstandingRequest::new(rejected, None));
                            break;
                        }
                    }
                    // BEP 6 Fast extension: this is an optional request
                    // hint. The picker still follows ordinary choke state,
                    // so accepting the message keeps interoperable peers
                    // alive without bypassing request admission controls.
                    Message::AllowedFast(_) => {}
                    Message::Unchoke => {
                        download_choked = false;
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Unchoked {
                                peer: addr,
                                id: peer_id,
                                download_generation,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::Interested => {
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Interested {
                                peer: addr,
                                id: peer_id,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::NotInterested => {
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::NotInterested {
                                peer: addr,
                                id: peer_id,
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Message::Request { piece, begin, length } => {
                        let block = BlockExtent::new(piece, begin, length);
                        let request = UploadRequest {
                            piece,
                            begin,
                            length,
                        };
                        let have_piece = upload.have_pieces.get(piece as usize).unwrap_or(false);
                        let decision = upload_obligations.on_request(block, have_piece);
                        if matches!(
                            decision,
                            InboundDecision::Reject(InboundRejectReason::QueueFull)
                                | InboundDecision::Drop(InboundRejectReason::QueueFull)
                        ) {
                            upload_request_drops = upload_request_drops.saturating_add(1);
                            if upload_request_drops == 1 || upload_request_drops.is_multiple_of(256)
                            {
                                debug!(
                                    component = "peer",
                                    operation = "queue_upload_request",
                                    peer = %addr,
                                    result = "refused",
                                    reason = "upload_queue_saturated",
                                    dropped = upload_request_drops,
                                    "refusing excess upload requests without terminating peer"
                                );
                            }
                        }
                        match decision {
                            InboundDecision::Queue => start_upload_reads(
                                &mut upload_reads,
                                &mut upload_obligations,
                                &upload,
                            ),
                            InboundDecision::Reject(_) => {
                                reject_upload_request(&mut peer_io, &upload, request).await?;
                            }
                            InboundDecision::Drop(_) | InboundDecision::Duplicate => {}
                        }
                    }
                    Message::Cancel { piece, begin, length } => {
                        let block = BlockExtent::new(piece, begin, length);
                        if let Some(reject) = upload_obligations.on_cancel(block) {
                            send_upload_obligation_reject(&mut peer_io, reject).await?;
                        }
                    }
                    Message::Choke => {
                        download_choked = true;
                        download_generation = download_generation.wrapping_add(1);
                        let choked_requests = take_choked_requests(
                            &mut outstanding,
                            upload.fast_negotiated,
                        );
                        if !send_choke_event(
                            &peer_event_tx,
                            addr,
                            peer_id,
                            download_generation,
                            upload.fast_negotiated,
                            choked_requests,
                            &mut outstanding,
                        )
                        .await
                        {
                            break;
                        }
                        warn!(
                            component = "peer",
                            operation = "handle_choke",
                            peer = %addr,
                            result = "choked",
                            "choked"
                        );
                    }
                    Message::KeepAlive => {}
                    Message::Extended { ext_id: EXT_HANDSHAKE_ID, payload } => {
                        match ExtensionHandshake::parse(&payload) {
                            Ok(handshake) => {
                                remote_ut_metadata_id = handshake.ut_metadata_id();
                                if !send_peer_event(
                                    &peer_event_tx,
                                    PeerEvent::ExtendedHandshake {
                                        peer: addr,
                                        id: peer_id,
                                        ut_metadata_id: handshake.ut_metadata_id(),
                                        ut_pex_id: handshake.ut_pex_id(),
                                        metadata_size: handshake.metadata_size,
                                    },
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                debug!(
                                    component = "peer",
                                    operation = "parse_extension_handshake",
                                    peer = %addr,
                                    result = "error",
                                    error = %e,
                                    "ignoring invalid extension handshake"
                                );
                            }
                        }
                    }
                    Message::Extended { ext_id: LOCAL_UT_METADATA_ID, payload } => {
                        match UtMetadataMessage::parse(&payload) {
                            Ok(UtMetadataMessage::Request { piece }) => {
                                let Some(remote_ut_metadata_id) = remote_ut_metadata_id else {
                                    debug!(
                                        component = "peer",
                                        operation = "send_ut_metadata",
                                        peer = %addr,
                                        result = "ignored",
                                        reason = "remote_extension_id_unknown",
                                        "ignoring metadata request before peer extension negotiation"
                                    );
                                    continue;
                                };
                                let response = upload
                                    .metadata
                                    .as_ref()
                                    .map(|metadata| metadata_response(piece, metadata))
                                    .unwrap_or(UtMetadataMessage::Reject { piece });
                                peer_io.send(Message::Extended {
                                    ext_id: remote_ut_metadata_id,
                                    payload: response.encode(),
                                }).await?;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                debug!(
                                    component = "peer",
                                    operation = "parse_ut_metadata",
                                    peer = %addr,
                                    result = "error",
                                    error = %e,
                                    "ignoring invalid ut_metadata message"
                                );
                            }
                        }
                    }
                    Message::Extended { ext_id: LOCAL_UT_PEX_ID, payload } => {
                        if !upload.pex_enabled || upload.is_private {
                            continue;
                        }
                        match parse_ut_pex_peers(&payload) {
                            Ok(pex) if !pex.added.is_empty() || !pex.dropped.is_empty() => {
                                let memory_bytes = pex
                                    .added
                                    .capacity()
                                    .saturating_add(pex.dropped.capacity())
                                    .saturating_mul(std::mem::size_of::<SocketAddr>());
                                let memory_lease = match reserve_peer_event_bytes(
                                    &upload.resources,
                                    memory_bytes,
                                ) {
                                    Ok(lease) => lease,
                                    Err(error) => {
                                        debug!(
                                            component = "peer",
                                            operation = "reserve_pex_event_memory",
                                            peer = %addr,
                                            result = "denied",
                                            error = %error,
                                            "disconnecting peer because its peer-exchange payload exceeds the memory budget"
                                        );
                                        break;
                                    }
                                };
                                if !send_peer_event(
                                    &peer_event_tx,
                                    PeerEvent::PeerExchange {
                                        peer: addr,
                                        id: peer_id,
                                        peers: pex.added,
                                        dropped: pex.dropped,
                                        _memory_lease: memory_lease,
                                    },
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                debug!(
                                    component = "peer",
                                    operation = "parse_ut_pex",
                                    peer = %addr,
                                    result = "error",
                                    error = %e,
                                    "ignoring invalid ut_pex message"
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            upload_result = upload_reads.next(), if !upload_reads.is_empty() => {
                let Some(upload_result) = upload_result else {
                    continue;
                };
                let (request, result, rate_interrupted) = match upload_result {
                    Ok(result) => result,
                    Err(error) => {
                        if error.is_cancelled() && upload_choked && !upload.fast_negotiated {
                            // Legacy Choke explicitly ended these in-service
                            // obligations and aborted their reads.
                            continue;
                        }
                        warn!(
                            component = "peer",
                            operation = "read_upload_block",
                            peer = %addr,
                            result = "worker_join_error",
                            error = %crate::task_join_error_summary(
                                "upload block read task",
                                &error
                            ),
                            "upload block worker failed"
                        );
                        // The JoinError no longer carries the request
                        // coordinate. End this connection so an accepted
                        // obligation cannot remain stranded in the owner.
                        upload_obligations.close();
                        break;
                    }
                };
                let block_extent = BlockExtent::new(request.piece, request.begin, request.length);
                match result {
                    Ok(_block) if rate_interrupted => {
                        // Rate admission runs with the bounded read task, so
                        // a slow limit cannot stop this loop from consuming
                        // peer input or control messages. If the peer remains
                        // unchoked, retry valid demand after the control wake;
                        // otherwise terminate it according to Fast semantics.
                        if upload_choked {
                            if upload_obligations.may_send(&block_extent) {
                                if !upload_obligations.requeue_front(block_extent) {
                                    anyhow::bail!("v1 allowed-fast upload could not be requeued");
                                }
                            } else {
                                finish_upload_obligation(
                                    &mut peer_io,
                                    &mut upload_obligations,
                                    block_extent,
                                    InboundServed::Discarded,
                                )
                                .await?;
                            }
                        } else if !upload_obligations.requeue_front(block_extent) {
                            anyhow::bail!("v1 upload obligation could not be requeued");
                        }
                    }
                    Ok(upload_block) if upload_obligations.may_send(&block_extent) => {
                        let bytes = upload_block.data.len() as u64;
                        peer_io
                            .send(Message::Piece {
                                piece: request.piece,
                                begin: request.begin,
                                data: upload_block.data,
                            })
                            .await?;
                        if !send_peer_event(
                            &peer_event_tx,
                            PeerEvent::Uploaded {
                                peer: addr,
                                id: peer_id,
                                bytes,
                            },
                        )
                        .await
                        {
                            break;
                        }
                        finish_upload_obligation(
                            &mut peer_io,
                            &mut upload_obligations,
                            block_extent,
                            InboundServed::Sent,
                        )
                        .await?;
                    }
                    Ok(_) => {
                        // A local choke or shutdown may have arrived while
                        // the read was in flight. Dropping the leased block
                        // is safer than sending data after the state change;
                        // a Fast peer is told the request will not be served.
                        finish_upload_obligation(
                            &mut peer_io,
                            &mut upload_obligations,
                            block_extent,
                            InboundServed::Discarded,
                        )
                        .await?;
                    }
                    Err(error) => {
                        warn!(
                            component = "peer",
                            operation = "read_upload_block",
                            peer = %addr,
                            piece = request.piece,
                            begin = request.begin,
                            length = request.length,
                            result = "error",
                            error = %error,
                            "failed to read upload block"
                        );
                        finish_upload_obligation(
                            &mut peer_io,
                            &mut upload_obligations,
                            block_extent,
                            InboundServed::Failed,
                        )
                        .await?;
                    }
                }
                start_upload_reads(
                    &mut upload_reads,
                    &mut upload_obligations,
                    &upload,
                );
            }
            _ = timeout_tick.tick() => {
                if last_activity.elapsed() > PEER_IDLE_TIMEOUT {
                    anyhow::bail!("peer idle timeout");
                }
                let timed_out = take_timed_out_requests(&mut outstanding, Duration::from_secs(60));
                if !timed_out.is_empty()
                    && !send_peer_event(
                        &peer_event_tx,
                        PeerEvent::RequestTimedOut {
                            peer: addr,
                            id: peer_id,
                            timed_out: timed_out.clone(),
                        },
                    )
                    .await
                {
                    outstanding.extend(
                        timed_out
                            .into_iter()
                            .map(|request| OutstandingRequest::new(request, None)),
                    );
                    break;
                }
            }
        }
        }
        Ok(())
    }
    .await;

    // Keep this outside the result-producing block: socket, framing, and
    // event-delivery errors can leave that block through `?`, bypassing the
    // normal loop break. Dropping a JoinHandle detaches its task, so every
    // in-flight upload read must be explicitly aborted on every exit path.
    upload_obligations.close();
    abort_upload_reads(&upload_reads);

    let outstanding = drain_outstanding(&mut outstanding);
    if send_terminal_event_on_peer_channel {
        let _ = send_peer_event(
            &peer_event_tx,
            PeerEvent::Disconnected {
                peer: addr,
                id: peer_id,
                outstanding: outstanding.clone(),
            },
        )
        .await;
    }
    PeerLoopExit {
        result,
        outstanding,
    }
}

async fn send_extension_handshake(
    peer_io: &mut PeerIo,
    metadata: Option<&Arc<Vec<u8>>>,
    is_private: bool,
    pex_enabled: bool,
    remote_supports_extension: bool,
) -> anyhow::Result<()> {
    if remote_supports_extension {
        let handshake = extension_handshake_for_torrent(metadata, is_private, pex_enabled);
        peer_io
            .send(Message::Extended {
                ext_id: EXT_HANDSHAKE_ID,
                payload: handshake.encode(),
            })
            .await?;
    }
    Ok(())
}

fn extension_handshake_for_torrent(
    metadata: Option<&Arc<Vec<u8>>>,
    is_private: bool,
    pex_enabled: bool,
) -> ExtensionHandshake {
    let metadata_size = metadata.and_then(|bytes| u32::try_from(bytes.len()).ok());
    let mut handshake = ExtensionHandshake::new(metadata_size);
    if metadata_size.is_some() {
        handshake = handshake.with_ut_metadata(LOCAL_UT_METADATA_ID);
    }
    if pex_enabled && !is_private {
        handshake = handshake.with_ut_pex(LOCAL_UT_PEX_ID);
    }
    handshake
}

fn metadata_response(piece: u32, metadata: &[u8]) -> UtMetadataMessage {
    let Some(start) = (piece as usize).checked_mul(METADATA_PIECE_SIZE) else {
        return UtMetadataMessage::Reject { piece };
    };
    if start >= metadata.len() {
        return UtMetadataMessage::Reject { piece };
    }
    let Some(end) = start
        .checked_add(METADATA_PIECE_SIZE)
        .map(|end| end.min(metadata.len()))
    else {
        return UtMetadataMessage::Reject { piece };
    };
    let Ok(total_size) = u32::try_from(metadata.len()) else {
        return UtMetadataMessage::Reject { piece };
    };
    UtMetadataMessage::Data {
        piece,
        total_size,
        data: metadata[start..end].to_vec(),
    }
}

#[derive(Debug)]
struct OutstandingRequest {
    req: BlockRequest,
    sent_at: Instant,
    /// Expected response capacity remains charged until the block event takes
    /// ownership or this request reaches a terminal lifecycle path.
    memory_lease: Option<MemoryLease>,
}

impl OutstandingRequest {
    fn new(req: BlockRequest, memory_lease: Option<MemoryLease>) -> Self {
        Self {
            req,
            sent_at: Instant::now(),
            memory_lease,
        }
    }
}

async fn send_have_state(
    peer_io: &mut PeerIo,
    have_pieces: &PieceBitmap,
    resources: &ResourceGovernor,
    remote_supports_fast: bool,
) -> anyhow::Result<()> {
    let total = have_pieces.len();
    let have = have_pieces.count_ones();
    // BEP 6 Fast extension: a peer that has everything or nothing can skip
    // the full bitfield entirely. Only take this path with peers that
    // negotiated Fast-extension support during the handshake; peers that
    // did not must keep getting exactly today's behavior (a normal
    // Bitfield, or nothing at all when we have zero pieces).
    if remote_supports_fast {
        if total > 0 && have == total {
            peer_io.send(Message::HaveAll).await?;
            return Ok(());
        }
        if have == 0 {
            peer_io.send(Message::HaveNone).await?;
            return Ok(());
        }
    } else if have == 0 {
        return Ok(());
    }
    let _wire_memory_lease = reserve_peer_bitfield_wire_bytes(resources, have_pieces)?;
    let bitfield = have_pieces.to_bitfield();
    peer_io.send(Message::Bitfield(bitfield)).await?;
    Ok(())
}

async fn read_upload_block(
    upload: &UploadReadContext,
    piece: u32,
    begin: u32,
    length: u32,
) -> anyhow::Result<LeasedUploadBlock> {
    if length == 0 || length > MAX_BLOCK_SIZE {
        anyhow::bail!("invalid upload block length {length}");
    }
    PieceGeometry::validate_served_block(
        &upload.piece_map,
        BlockExtent::new(piece, begin, length),
    )?;
    let lease = reserve_peer_upload_bytes(&upload.resources, length)?;
    let regions = upload.piece_map.validate_request(piece, begin, length)?;
    let mut data = Vec::with_capacity(length as usize);
    for region in regions {
        if region.pad {
            data.resize(data.len() + region.length as usize, 0);
            continue;
        }
        let path = region.path.resolve(&upload.save_root);
        let read = scheduled_read_owned(
            &upload.storage,
            IoClass::PeerRead,
            &path,
            region.file_offset,
            region.length as usize,
        )
        .await?;
        data.extend_from_slice(read.as_slice());
    }
    if data.len() != length as usize {
        anyhow::bail!(
            "upload block read assembled {} bytes, expected {}",
            data.len(),
            length
        );
    }
    Ok(LeasedUploadBlock {
        data: bytes::Bytes::from(data),
        _lease: lease,
    })
}

/// Answer a request we will not serve. With Fast negotiated BEP 6 requires an
/// explicit Reject Request; without it the legacy behaviour (silence) stands.
async fn reject_upload_request(
    peer_io: &mut PeerIo,
    upload: &UploadContext,
    request: UploadRequest,
) -> anyhow::Result<()> {
    if upload.fast_negotiated {
        peer_io
            .send(Message::Reject {
                piece: request.piece,
                begin: request.begin,
                length: request.length,
            })
            .await?;
    }
    Ok(())
}

async fn send_upload_obligation_reject(
    peer_io: &mut PeerIo,
    reject: InboundReject,
) -> anyhow::Result<()> {
    let block = reject.block;
    peer_io
        .send(Message::Reject {
            piece: block.piece,
            begin: block.begin,
            length: block.length,
        })
        .await?;
    Ok(())
}

async fn finish_upload_obligation(
    peer_io: &mut PeerIo,
    obligations: &mut InboundObligations,
    block: BlockExtent,
    served: InboundServed,
) -> anyhow::Result<()> {
    if let Some(reject) = obligations.finish(block, served) {
        send_upload_obligation_reject(peer_io, reject).await?;
    }
    Ok(())
}

fn start_upload_reads(
    upload_reads: &mut futures::stream::FuturesUnordered<tokio::task::JoinHandle<UploadReadResult>>,
    obligations: &mut InboundObligations,
    upload: &UploadContext,
) {
    let read_context = UploadReadContext {
        save_root: upload.save_root.clone(),
        piece_map: upload.piece_map.clone(),
        storage: upload.storage.clone(),
        resources: upload.resources.clone(),
        torrent_upload: Arc::clone(&upload.torrent_upload),
        global_upload: Arc::clone(&upload.global_upload),
        upload_control: upload.upload_control.clone(),
    };
    while upload_reads.len() < MAX_PENDING_UPLOAD_READS {
        let Some(block) = obligations.next_request() else {
            break;
        };
        let request = UploadRequest {
            piece: block.piece,
            begin: block.begin,
            length: block.length,
        };
        let read_context_for_task = read_context.clone();
        upload_reads.push(tokio::spawn(async move {
            let result = match timeout(
                PEER_UPLOAD_READ_TIMEOUT,
                read_upload_block(
                    &read_context_for_task,
                    request.piece,
                    request.begin,
                    request.length,
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!(
                    "upload block read exceeded its peer deadline ({}s)",
                    PEER_UPLOAD_READ_TIMEOUT.as_secs()
                )),
            };
            let (result, rate_interrupted) = match result {
                Ok(block) => {
                    let bytes = block.data.len() as u64;
                    let control_generation = read_context_for_task.upload_control.generation();
                    let admitted = read_context_for_task
                        .torrent_upload
                        .acquire_or_cancelled(
                            bytes,
                            &read_context_for_task.upload_control,
                            control_generation,
                        )
                        .await
                        && read_context_for_task
                            .global_upload
                            .acquire_or_cancelled(
                                bytes,
                                &read_context_for_task.upload_control,
                                control_generation,
                            )
                            .await
                        && read_context_for_task.upload_control.generation() == control_generation;
                    (Ok(block), !admitted)
                }
                Err(error) => (Err(error), false),
            };
            (request, result, rate_interrupted)
        }));
    }
}

fn abort_upload_reads(
    upload_reads: &futures::stream::FuturesUnordered<tokio::task::JoinHandle<UploadReadResult>>,
) {
    for read in upload_reads.iter() {
        read.abort();
    }
}

fn validate_bitfield_shape(bits: &[u8], piece_count: usize) -> anyhow::Result<()> {
    let expected_len = piece_count.div_ceil(8);
    if bits.len() != expected_len {
        anyhow::bail!(
            "bitfield length {} does not match expected {}",
            bits.len(),
            expected_len
        );
    }
    if !piece_count.is_multiple_of(8) && !bits.is_empty() {
        let used_bits = piece_count % 8;
        let spare_mask = (1u8 << (8 - used_bits)) - 1;
        if bits[bits.len() - 1] & spare_mask != 0 {
            anyhow::bail!("bitfield has non-zero spare bits");
        }
    }
    Ok(())
}

#[cfg(test)]
fn bitfield_to_pieces(bits: &[u8], piece_count: usize) -> anyhow::Result<Vec<bool>> {
    validate_bitfield_shape(bits, piece_count)?;

    let mut pieces = Vec::with_capacity(piece_count);
    for byte in bits {
        for bit in (0..8).rev() {
            if pieces.len() == piece_count {
                return Ok(pieces);
            }
            pieces.push((byte & (1 << bit)) != 0);
        }
    }
    Ok(pieces)
}

#[cfg(test)]
fn pieces_to_bitfield(pieces: &[bool]) -> Vec<u8> {
    let mut bits = vec![0u8; pieces.len().div_ceil(8)];
    for (idx, has_piece) in pieces.iter().copied().enumerate() {
        if has_piece {
            bits[idx / 8] |= 0x80 >> (idx % 8);
        }
    }
    bits
}

fn super_seed_visible_pieces<A: PieceAvailability + ?Sized>(
    have_pieces: &A,
    piece_count: usize,
    peer_addr: SocketAddr,
) -> PieceBitmap {
    let available_count = (0..piece_count)
        .filter(|piece| have_pieces.has_piece(*piece))
        .count();
    let mut visible = PieceBitmap::new(piece_count);
    if available_count == 0 {
        return visible;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    peer_addr.hash(&mut hasher);
    let target = (hasher.finish() as usize) % available_count;
    if let Some(selected) = (0..piece_count)
        .filter(|piece| have_pieces.has_piece(*piece))
        .nth(target)
    {
        visible.set(selected, true);
    }
    visible
}

fn take_matching_outstanding(
    outstanding: &mut Vec<OutstandingRequest>,
    piece: u32,
    begin: u32,
    length: u32,
) -> Option<OutstandingRequest> {
    let pos = outstanding.iter().position(|out| {
        out.req.piece == piece && out.req.begin == begin && out.req.length == length
    })?;
    Some(outstanding.swap_remove(pos))
}

fn remove_requested_block(requested: &mut Vec<BlockRequest>, piece: u32, begin: u32) {
    if let Some(pos) = requested
        .iter()
        .position(|req| req.piece == piece && req.begin == begin)
    {
        requested.swap_remove(pos);
    }
}

fn take_timed_out_requests(
    outstanding: &mut Vec<OutstandingRequest>,
    timeout: Duration,
) -> Vec<BlockRequest> {
    let now = Instant::now();
    let mut timed_out = Vec::new();
    let mut idx = 0;
    while idx < outstanding.len() {
        if now.duration_since(outstanding[idx].sent_at) >= timeout {
            timed_out.push(outstanding.swap_remove(idx).req);
        } else {
            idx += 1;
        }
    }
    timed_out
}

fn drain_outstanding(outstanding: &mut Vec<OutstandingRequest>) -> Vec<BlockRequest> {
    outstanding.drain(..).map(|out| out.req).collect()
}

fn take_choked_requests(
    outstanding: &mut Vec<OutstandingRequest>,
    fast_negotiated: bool,
) -> Vec<BlockRequest> {
    if fast_negotiated {
        outstanding.iter().map(|request| request.req).collect()
    } else {
        drain_outstanding(outstanding)
    }
}

fn webseed_retry_delay(failures: u8) -> Duration {
    let shift = failures.saturating_sub(1).min(9) as u32;
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    WEBSEED_RETRY_BASE
        .checked_mul(multiplier)
        .unwrap_or(WEBSEED_RETRY_MAX)
        .min(WEBSEED_RETRY_MAX)
}

fn webseed_error_for_log(error: &anyhow::Error) -> String {
    rt_tracker::sanitize_tracker_message(&error.to_string())
}
