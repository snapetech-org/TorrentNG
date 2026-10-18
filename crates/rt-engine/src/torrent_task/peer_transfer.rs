//! Peer protocol events, block scheduling, piece assembly, and transfer accounting.
//! Torrent state and storage completion remain ordered by the task actor.

use crate::network_budget::{refilled_bytes, token_refill_delay};
use crate::transfer::window::rate_credit_blocks;

use super::*;

impl TorrentTask {
    pub(super) fn discard_registered_peer(&mut self, peer: SocketAddr) {
        let Some(handle) = self.active_peers.remove(&peer) else {
            return;
        };
        remove_peer_availability(&mut self.picker.availability, &handle.peer_has);
        for request in handle.requested {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
        }
        self.transfer_ledger.disconnect(handle.transfer_conn);
    }

    pub(super) fn ensure_transfer_piece_memory(
        ledger: &TransferLedger<Arc<PieceMap>>,
        leases: &mut HashMap<u32, MemoryLease>,
        resources: &ResourceGovernor,
        piece: u32,
    ) -> bool {
        if ledger.piece_progress(piece).is_some() {
            return true;
        }
        let Some(bytes) = ledger.estimated_piece_state_bytes(piece) else {
            return false;
        };
        let Ok(bytes) = u64::try_from(bytes) else {
            return false;
        };
        if leases.try_reserve(1).is_err() {
            return false;
        }
        let Some(lease) = resources.try_acquire(MemoryClass::PeerBuffer, bytes) else {
            return false;
        };
        leases.insert(piece, lease);
        true
    }

    pub(super) fn restore_partial_transfer_state(
        &mut self,
        piece: u32,
        received_blocks: &[u32],
    ) -> bool {
        if self.restored_partial_pieces.contains(&piece) {
            return true;
        }
        let piece_index = piece as usize;
        self.picker
            .restore_partial_piece(piece_index, received_blocks);
        if !self.picker.is_piece_in_progress(piece_index) {
            return false;
        }

        let Some(bytes) = self.transfer_ledger.estimated_piece_state_bytes(piece) else {
            self.picker.reject_piece(piece_index);
            return false;
        };
        let Ok(bytes) = u64::try_from(bytes) else {
            self.picker.reject_piece(piece_index);
            return false;
        };
        if self.transfer_piece_memory_leases.try_reserve(1).is_err()
            || self.restored_partial_pieces.try_reserve(1).is_err()
        {
            self.picker.reject_piece(piece_index);
            return false;
        }
        let Some(lease) = self.resources.try_acquire(MemoryClass::PeerBuffer, bytes) else {
            self.picker.reject_piece(piece_index);
            return false;
        };
        match self
            .transfer_ledger
            .restore_partial_blocks(piece, received_blocks)
        {
            Ok(true) => {
                self.transfer_piece_memory_leases.insert(piece, lease);
                self.restored_partial_pieces.insert(piece);
                true
            }
            Ok(false) => {
                self.picker.reject_piece(piece_index);
                false
            }
            Err(error) => {
                debug!(
                    component = "transfer",
                    operation = "restore_partial_piece",
                    torrent = %self.info_hash_hex,
                    piece,
                    result = "discarded",
                    reason = ?error,
                    "fastresume partial piece could not enter the shared ledger; its blocks will be re-requested"
                );
                self.picker.reject_piece(piece_index);
                false
            }
        }
    }

    fn release_transfer_piece_memory(&mut self, piece: u32) {
        self.transfer_piece_memory_leases.remove(&piece);
    }

    pub(super) fn apply_transfer_actions(&mut self, actions: Vec<TransferAction>) {
        let mut queue_full = 0u64;
        for action in actions {
            let TransferAction::Cancel { conn, block } = action;
            if self.webseed_transfer_conn == Some(conn) {
                // Mark-have and epoch changes can revoke a webseed block just
                // like a peer request. Abort its bounded HTTP owner and drop
                // the ledger generation so a late body cannot reach storage.
                self.close_webseed_transfer_connection();
                self.picker
                    .cancel_request(block.piece as usize, block.begin);
                continue;
            }
            let Some(handle) = self
                .active_peers
                .values()
                .find(|handle| handle.transfer_conn == conn)
            else {
                continue;
            };
            let request = BlockRequest {
                piece: block.piece,
                begin: block.begin,
                length: block.length,
            };
            if handle
                .cmd_tx
                .try_send(PeerCommand::Cancel { request })
                .is_err()
            {
                queue_full = queue_full.saturating_add(1);
            }
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    fn transfer_now(&self) -> u64 {
        self.transfer_started_at
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64
    }

    fn ensure_webseed_transfer_connection(&mut self) -> Option<ConnectionId> {
        if let Some(conn) = self.webseed_transfer_conn {
            return Some(conn);
        }
        let bytes = u64::try_from(self.transfer_ledger.connection_state_bytes()).ok()?;
        let memory_lease = self.resources.try_acquire(MemoryClass::PeerBuffer, bytes)?;
        let conn = self.transfer_ledger.try_connect(1)?;
        self.webseed_transfer_conn = Some(conn);
        self._webseed_transfer_connection_memory_lease = Some(memory_lease);
        Some(conn)
    }

    pub(super) fn reserve_webseed_block(&mut self, request: BlockRequest) -> Option<ConnectionId> {
        let Some(conn) = self.ensure_webseed_transfer_connection() else {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            return None;
        };
        if !Self::ensure_transfer_piece_memory(
            &self.transfer_ledger,
            &mut self.transfer_piece_memory_leases,
            &self.resources,
            request.piece,
        ) {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            self.close_webseed_transfer_connection();
            return None;
        }
        let block = BlockExtent::new(request.piece, request.begin, request.length);
        if let Err(error) = self
            .transfer_ledger
            .reserve(conn, block, &ReserveMode::Exclusive)
        {
            self.transfer_ledger.discard_empty_piece(request.piece);
            if self.transfer_ledger.piece_progress(request.piece).is_none() {
                self.transfer_piece_memory_leases.remove(&request.piece);
            }
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            self.close_webseed_transfer_connection();
            debug!(
                component = "transfer",
                operation = "reserve_webseed_block",
                torrent = %self.info_hash_hex,
                piece = request.piece,
                begin = request.begin,
                result = "deferred",
                reason = ?error,
                "shared transfer ledger refused a webseed block"
            );
            return None;
        }
        Some(conn)
    }

    pub(super) fn commit_webseed_block(
        &mut self,
        conn: ConnectionId,
        request: BlockRequest,
    ) -> bool {
        let block = BlockExtent::new(request.piece, request.begin, request.length);
        let now = self.transfer_now();
        let (outcome, actions) = self.transfer_ledger.commit(conn, block, now);
        self.apply_transfer_actions(actions);
        outcome == CommitOutcome::OnWire
    }

    pub(super) fn release_webseed_block(&mut self, conn: ConnectionId, request: BlockRequest) {
        let block = BlockExtent::new(request.piece, request.begin, request.length);
        self.transfer_ledger.on_reject(conn, block);
        self.picker
            .cancel_request(request.piece as usize, request.begin);
        self.transfer_ledger.discard_empty_piece(request.piece);
        if self.transfer_ledger.piece_progress(request.piece).is_none() {
            self.transfer_piece_memory_leases.remove(&request.piece);
        }
    }

    pub(super) fn close_webseed_transfer_connection(&mut self) {
        if let Some(fetch) = self.webseed_fetch.take() {
            fetch.task.abort();
            self.reconcile_webseed_local_tokens(
                fetch.reserved_local_bytes,
                fetch.local_tokens_reserved,
                fetch.observed_network_bytes.load(Ordering::Acquire),
            );
            self.picker
                .cancel_request(fetch.request.piece as usize, fetch.request.begin);
        }
        let Some(conn) = self.webseed_transfer_conn.take() else {
            self._webseed_transfer_connection_memory_lease = None;
            return;
        };
        for block in self.transfer_ledger.disconnect(conn) {
            self.picker
                .cancel_request(block.piece as usize, block.begin);
            self.transfer_ledger.discard_empty_piece(block.piece);
            if self.transfer_ledger.piece_progress(block.piece).is_none() {
                self.transfer_piece_memory_leases.remove(&block.piece);
            }
        }
        self._webseed_transfer_connection_memory_lease = None;
    }

    pub(super) async fn handle_webseed_block(&mut self, event: LeasedBlockEvent) {
        if let Some(conn) = self.webseed_transfer_conn {
            self.handle_block_with_owner(event, Some(conn), None).await;
        } else {
            self.handle_block(event).await;
        }
        self.close_webseed_transfer_connection();
    }

    pub(super) async fn handle_peer_event(&mut self, event: PeerEvent) {
        let (peer, id) = event.identity();
        if !self
            .active_peers
            .get(&peer)
            .is_some_and(|handle| handle.id == id)
        {
            debug!(
                component = "peer",
                operation = "handle_event",
                torrent = %self.info_hash_hex,
                peer = %peer,
                peer_id = ?id,
                result = "ignored",
                reason = "stale_connection",
                "dropping an event from a replaced peer connection"
            );
            return;
        }
        match event {
            PeerEvent::Bitfield {
                peer,
                pieces,
                _memory_lease,
                ..
            } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    reconcile_peer_availability(
                        &mut self.picker.availability,
                        &handle.peer_has,
                        &pieces,
                    );
                    handle.peer_has = pieces;
                }
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::Have { peer, piece, .. } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    if !handle.peer_has.get(piece as usize).unwrap_or(false) {
                        handle.peer_has.set(piece as usize, true);
                        self.picker.availability.add_have(piece as usize);
                    }
                }
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::Unchoked {
                peer,
                download_generation,
                ..
            } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.download_generation = download_generation;
                    handle.choked = false;
                }
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::Choked {
                peer,
                download_generation,
                fast_negotiated,
                outstanding,
                ..
            } => {
                let mut not_sent = Vec::new();
                let transfer_conn = self
                    .active_peers
                    .get(&peer)
                    .map(|handle| handle.transfer_conn);
                let outstanding = if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.download_generation = download_generation;
                    handle.choked = true;
                    let requested = std::mem::take(&mut handle.requested);
                    if fast_negotiated {
                        let (on_wire, unsent) =
                            Self::reconcile_fast_choke_requests(requested, outstanding);
                        handle.outstanding = on_wire.len();
                        handle.requested = on_wire;
                        not_sent.extend(unsent.iter().copied());
                        unsent
                    } else {
                        handle.outstanding = 0;
                        Self::merge_unique_block_requests(requested, outstanding)
                    }
                } else {
                    outstanding
                };
                if let Some(transfer_conn) = transfer_conn {
                    if fast_negotiated {
                        for request in not_sent {
                            self.transfer_ledger.on_reject(
                                transfer_conn,
                                BlockExtent::new(request.piece, request.begin, request.length),
                            );
                        }
                    } else {
                        self.transfer_ledger.on_choke(transfer_conn, false);
                    }
                }
                for req in outstanding {
                    self.picker.cancel_request(req.piece as usize, req.begin);
                }
            }
            PeerEvent::Interested { peer, .. } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.interested = true;
                }
            }
            PeerEvent::NotInterested { peer, .. } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.interested = false;
                }
            }
            PeerEvent::RequestSent { peer, request, .. } => {
                let Some(transfer_conn) = self
                    .active_peers
                    .get(&peer)
                    .map(|handle| handle.transfer_conn)
                else {
                    return;
                };
                let now = self.transfer_now();
                let (outcome, actions) = self.transfer_ledger.commit(
                    transfer_conn,
                    BlockExtent::new(request.piece, request.begin, request.length),
                    now,
                );
                if outcome == CommitOutcome::Orphaned {
                    self.apply_transfer_actions(actions);
                }
            }
            PeerEvent::Piece { peer, block, .. } => {
                // A peer task can race shutdown and leave one final block in
                // the bounded event channel. Once its handle is gone, that
                // block is stale and must not be written after a recheck or a
                // storage move has switched the torrent's filesystem state.
                let Some(transfer_conn) = self
                    .active_peers
                    .get(&peer)
                    .map(|handle| handle.transfer_conn)
                else {
                    return;
                };
                let Some(handle) = self.active_peers.get_mut(&peer) else {
                    return;
                };
                handle.outstanding = handle.outstanding.saturating_sub(1);
                remove_requested_block(
                    &mut handle.requested,
                    block.block.piece,
                    block.block.offset,
                );
                record_peer_transfer(handle, false, block.len() as u64);
                self.handle_block_with_owner(block, Some(transfer_conn), Some(peer))
                    .await;
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::RequestRejected { peer, rejected, .. } => {
                if let Some(transfer_conn) = self
                    .active_peers
                    .get(&peer)
                    .map(|handle| handle.transfer_conn)
                {
                    self.transfer_ledger.on_reject(
                        transfer_conn,
                        BlockExtent::new(rejected.piece, rejected.begin, rejected.length),
                    );
                }
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.outstanding = handle.outstanding.saturating_sub(1);
                    remove_requested_block(&mut handle.requested, rejected.piece, rejected.begin);
                }
                self.picker
                    .cancel_request(rejected.piece as usize, rejected.begin);
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::Uploaded { peer, bytes, .. } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    record_peer_transfer(handle, true, bytes);
                }
                self.record_upload(bytes).await;
            }
            PeerEvent::Disconnected {
                peer, outstanding, ..
            } => {
                // The peer loop's view can omit commands still queued in its
                // mailbox. Union it with the engine's view so a disconnect
                // cannot strand picker reservations, while deduplicating the
                // same coordinate reported by both sides.
                let outstanding = if let Some(handle) = self.active_peers.get_mut(&peer) {
                    let requested = std::mem::take(&mut handle.requested);
                    Self::merge_unique_block_requests(requested, outstanding)
                } else {
                    outstanding
                };
                if let Some(handle) = self.active_peers.remove(&peer) {
                    remove_peer_availability(&mut self.picker.availability, &handle.peer_has);
                    self.transfer_ledger.disconnect(handle.transfer_conn);
                }
                for req in outstanding {
                    self.picker.cancel_request(req.piece as usize, req.begin);
                }
            }
            PeerEvent::RequestTimedOut {
                peer, timed_out, ..
            } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.outstanding = handle.outstanding.saturating_sub(timed_out.len());
                    for req in &timed_out {
                        remove_requested_block(&mut handle.requested, req.piece, req.begin);
                    }
                }
                for req in timed_out {
                    if let Some(transfer_conn) = self
                        .active_peers
                        .get(&peer)
                        .map(|handle| handle.transfer_conn)
                    {
                        self.transfer_ledger.on_reject(
                            transfer_conn,
                            BlockExtent::new(req.piece, req.begin, req.length),
                        );
                    }
                    self.picker.cancel_request(req.piece as usize, req.begin);
                }
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::ExtendedHandshake {
                peer,
                ut_metadata_id,
                ut_pex_id,
                metadata_size,
                reqq,
                ..
            } => {
                if let Some(handle) = self.active_peers.get_mut(&peer) {
                    handle.ut_metadata_id = ut_metadata_id;
                    handle.ut_pex_id = ut_pex_id;
                    handle.metadata_size = metadata_size;
                    handle.request_window.set_remote_reqq(reqq);
                }
                self.refill_peer_requests(peer).await;
            }
            PeerEvent::PeerExchange {
                peer,
                peers,
                dropped,
                ..
            } => {
                if self.meta.private {
                    return;
                }
                let peer_count = peers.len();
                self.remember_tracker_peers(&peers);
                // PEX's dropped list is advisory: remove stale retry
                // candidates, but do not forcibly tear down a connection that
                // may still be valid from our side.
                for dropped_peer in dropped {
                    self.known_tracker_peers.remove(&dropped_peer);
                }
                self.connect_peers(peers, PeerSource::PeerExchange).await;
                debug!(
                    torrent = %self.info_hash_hex,
                    peer = %peer,
                    peers = peer_count,
                    "peer exchange discovered peers"
                );
            }
        }
    }

    pub(super) async fn run_choker(&mut self) {
        let snapshots: Vec<PeerSnapshot> = self
            .active_peers
            .values()
            .map(|peer| PeerSnapshot {
                id: peer.id,
                interested: peer.interested,
                // An idle peer's last sample must not keep earning a slot:
                // the same staleness decay as the displayed rate applies.
                upload_rate: peer_rate(peer.upload_rate, peer.upload_rate_window_started) as f64,
                download_rate: peer_rate(peer.download_rate, peer.download_rate_window_started)
                    as f64,
                current_choke: if peer.upload_choked {
                    ChokeState::Choked
                } else {
                    ChokeState::Unchoked
                },
            })
            .collect();

        // Reciprocate while downloading; serve the fastest peers once
        // complete.
        let mode = if self.picker.is_complete() {
            ChokeMode::Seeding
        } else {
            ChokeMode::Downloading
        };
        let decisions = self.choker.run_with_mode(&snapshots, mode);
        let peers: Vec<SocketAddr> = self.active_peers.keys().copied().collect();
        let mut queue_full = 0u64;
        for addr in peers {
            let Some(handle) = self.active_peers.get_mut(&addr) else {
                continue;
            };
            if let Some(decision) = decisions.get(&handle.id).copied() {
                let (upload_choked, delivery_failed) = Self::try_apply_choke_decision(
                    &handle.cmd_tx,
                    &handle.upload_control,
                    handle.upload_choked,
                    decision,
                );
                handle.upload_choked = upload_choked;
                if delivery_failed {
                    queue_full = queue_full.saturating_add(1);
                }
            }
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    /// Apply the local choke state only after the command has entered the
    /// bounded peer mailbox. A full mailbox is a delivery failure, not a
    /// successful protocol transition; leaving the old state intact lets the
    /// next choker pass retry the command instead of permanently lying about
    /// what the remote peer received.
    pub(super) fn try_apply_choke_decision(
        tx: &mpsc::Sender<PeerCommand>,
        control: &RateLimitCancellation,
        currently_choked: bool,
        decision: ChokeDecision,
    ) -> (bool, bool) {
        let (desired, command) = match decision {
            ChokeDecision::Unchoke if currently_choked => (false, PeerCommand::Unchoke),
            ChokeDecision::Choke if !currently_choked => (true, PeerCommand::Choke),
            _ => return (currently_choked, false),
        };
        match tx.try_send(command) {
            Ok(()) => {
                control.cancel();
                (desired, false)
            }
            Err(_) => (currently_choked, true),
        }
    }

    pub(super) async fn refill_peer_requests(&mut self, peer: SocketAddr) {
        self.refill_download_tokens();
        let mut download_tokens = self.download_tokens;
        let download_limited = self.download_limit_bytes_per_sec.is_some();
        let other_peers_outstanding: usize = self
            .active_peers
            .iter()
            .filter(|(addr, _)| **addr != peer)
            .map(|(_, other)| other.outstanding)
            .sum();
        let Some(handle) = self.active_peers.get(&peer) else {
            return;
        };
        if handle.choked {
            return;
        }
        let transfer_conn = handle.transfer_conn;
        let download_rate = handle.download_rate;
        let download_rate_started = handle.download_rate_window_started;
        let peer_buffer = self.resources.snapshot().classes[MemoryClass::PeerBuffer as usize];
        let buffer_blocks = usize::try_from(
            peer_buffer.cap_bytes.saturating_sub(peer_buffer.used_bytes)
                / u64::from(MAX_BLOCK_SIZE),
        )
        .unwrap_or(usize::MAX);
        let torrent_credit_blocks =
            rate_credit_blocks(download_limited.then_some(download_tokens), MAX_BLOCK_SIZE);
        let global_credit_blocks = rate_credit_blocks(
            self.network_budget.download().available_credit_bytes(),
            MAX_BLOCK_SIZE,
        );
        let credit_blocks = torrent_credit_blocks.min(global_credit_blocks);
        let (device_queue_capacity, device_queue_available) = self.storage.device_queue_load();
        let device_backlog = if device_queue_capacity == 0 {
            f64::NAN
        } else {
            device_queue_capacity.saturating_sub(device_queue_available) as f64
                / device_queue_capacity as f64
        };
        let controlled_window = handle.request_window.window(&WindowInputs {
            useful_missing: usize::MAX,
            outstanding: handle.outstanding,
            credit_blocks,
            buffer_blocks,
            device_backlog,
        });

        let memory_pipeline = memory_aware_request_pipeline(
            self.piece_assembly_bytes,
            self.piece_assembly_soft_cap_bytes,
        );
        let request_pipeline = adaptive_request_pipeline(
            memory_pipeline,
            peer_rate(download_rate, download_rate_started) as f64,
            other_peers_outstanding,
        )
        .min(controlled_window);

        let mut queue_full = 0u64;
        let mut memory_retry = false;
        // Bytes the bucket was short by when refilling had to stop.
        let mut tokens_missing = None;
        {
            let picker = &mut self.picker;
            let peers = &mut self.active_peers;
            let transfer_ledger = &mut self.transfer_ledger;
            let transfer_piece_memory_leases = &mut self.transfer_piece_memory_leases;
            let resources = &self.resources;
            let Some(handle) = peers.get_mut(&peer) else {
                return;
            };
            while handle.outstanding < request_pipeline {
                let (req, mode) = match picker.pick(&handle.peer_has) {
                    Some(req) => (req, ReserveMode::Exclusive),
                    None => {
                        let Some(req) = picker.pick_endgame(&handle.peer_has, &handle.requested)
                        else {
                            break;
                        };
                        (req, ReserveMode::Endgame)
                    }
                };
                if download_limited && download_tokens < u64::from(req.length) {
                    picker.cancel_request(req.piece as usize, req.begin);
                    tokens_missing = Some(u64::from(req.length) - download_tokens);
                    break;
                }
                let Some(memory_lease) =
                    resources.try_acquire(MemoryClass::PeerBuffer, u64::from(req.length))
                else {
                    memory_retry = true;
                    picker.cancel_request(req.piece as usize, req.begin);
                    break;
                };
                if !Self::ensure_transfer_piece_memory(
                    transfer_ledger,
                    transfer_piece_memory_leases,
                    resources,
                    req.piece,
                ) {
                    memory_retry = true;
                    picker.cancel_request(req.piece as usize, req.begin);
                    break;
                }
                let block = BlockExtent::new(req.piece, req.begin, req.length);
                if let Err(error) = transfer_ledger.reserve(transfer_conn, block, &mode) {
                    picker.cancel_request(req.piece as usize, req.begin);
                    if transfer_ledger.piece_progress(req.piece).is_none() {
                        transfer_piece_memory_leases.remove(&req.piece);
                    }
                    debug!(
                        component = "transfer",
                        operation = "reserve_download_block",
                        torrent = %self.info_hash_hex,
                        peer = %peer,
                        piece = req.piece,
                        begin = req.begin,
                        result = "deferred",
                        reason = ?error,
                        "shared transfer ledger refused a picker candidate"
                    );
                    break;
                }
                if handle
                    .cmd_tx
                    .try_send(PeerCommand::Request {
                        request: req,
                        download_generation: handle.download_generation,
                        memory_lease,
                    })
                    .is_err()
                {
                    transfer_ledger.on_reject(transfer_conn, block);
                    queue_full = queue_full.saturating_add(1);
                    picker.cancel_request(req.piece as usize, req.begin);
                    break;
                }
                if download_limited {
                    download_tokens = download_tokens.saturating_sub(u64::from(req.length));
                }
                handle.outstanding += 1;
                handle.requested.push(req);
            }
        }
        if download_limited {
            self.download_tokens = download_tokens;
        }
        if memory_retry {
            self.peer_request_memory_retry_pending = true;
        }
        if memory_pipeline < PEER_REQUEST_PIPELINE_NORMAL {
            self.peer_request_window_reductions =
                self.peer_request_window_reductions.saturating_add(1);
        }
        if let (Some(missing), Some(limit)) = (tokens_missing, self.download_limit_bytes_per_sec) {
            // Wake exactly when the missing bytes have refilled, so a rate
            // below one block per second still makes progress.
            let wait = token_refill_delay(missing, limit);
            let deadline = Instant::now() + wait;
            self.download_token_wake = Some(
                self.download_token_wake
                    .map_or(deadline, |current| current.min(deadline)),
            );
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    pub(super) async fn retry_memory_denied_peer_requests(&mut self) {
        if !self.peer_request_memory_retry_pending {
            return;
        }
        self.peer_request_memory_retry_pending = false;
        if self.paused {
            return;
        }
        let peers: Vec<SocketAddr> = self.active_peers.keys().copied().collect();
        for peer in peers {
            self.refill_peer_requests(peer).await;
        }
    }

    pub(super) fn ensure_pending_transfer_piece(&mut self, piece: u32) -> &mut PendingPieceWork {
        if !self.pending_transfer_pieces.contains_key(&piece) {
            self.next_transfer_generation = self
                .next_transfer_generation
                .checked_add(1)
                .expect("torrent transfer-attempt generation exhausted");
            let generation = self.next_transfer_generation;
            self.pending_transfer_pieces.insert(
                piece,
                PendingPieceWork {
                    generation,
                    ..PendingPieceWork::default()
                },
            );
        }
        self.pending_transfer_pieces
            .get_mut(&piece)
            .expect("transfer work was inserted or already existed")
    }

    pub(super) async fn handle_block(&mut self, event: LeasedBlockEvent) {
        self.handle_block_with_owner(event, None, None).await;
    }

    async fn handle_block_with_owner(
        &mut self,
        event: LeasedBlockEvent,
        transfer_conn: Option<ConnectionId>,
        peer: Option<SocketAddr>,
    ) {
        let piece = event.block.piece;
        let offset = event.block.offset;
        let block_length = u32::try_from(event.block.data.len()).unwrap_or(u32::MAX);
        let transfer_block = BlockExtent::new(piece, offset, block_length);
        let accepted_at = self.transfer_now();
        let response_delay = event.response_delay_nanos;
        if !self.picker.is_piece_in_progress(piece as usize) {
            if let Some(conn) = transfer_conn {
                self.transfer_ledger.on_reject(conn, transfer_block);
            }
            debug!(
                component = "torrent",
                operation = "receive_block",
                torrent = %self.info_hash_hex,
                piece,
                offset,
                result = "ignored",
                reason = "piece_not_in_progress",
                "dropping a late block after the piece request state was released"
            );
            return;
        }
        if self.picker.is_block_received(piece as usize, offset)
            || self.pending_transfer_blocks.contains_key(&(piece, offset))
        {
            if let Some(conn) = transfer_conn {
                self.transfer_ledger.on_reject(conn, transfer_block);
            }
            debug!(
                component = "torrent",
                operation = "receive_block",
                torrent = %self.info_hash_hex,
                piece,
                offset,
                result = "ignored",
                reason = "duplicate_block",
                "dropping a duplicate block already accepted for this piece"
            );
            return;
        }
        if self
            .pending_transfer_pieces
            .get(&piece)
            .is_some_and(|work| work.failed)
        {
            if let Some(conn) = transfer_conn {
                self.transfer_ledger.on_reject(conn, transfer_block);
            }
            self.picker.cancel_request(piece as usize, offset);
            return;
        }
        if let Err(error) =
            validate_padding_block(&self.piece_map, piece, offset, &event.block.data)
        {
            warn!(
                component = "torrent",
                operation = "receive_block",
                torrent = %self.info_hash_hex,
                piece,
                offset,
                result = "rejected",
                error = %error,
                "peer supplied non-zero or malformed BEP 47 padding data"
            );
            if let Some(conn) = transfer_conn {
                self.transfer_ledger.on_reject(conn, transfer_block);
            }
            self.picker.cancel_request(piece as usize, offset);
            let has_in_flight_writes =
                if let Some(work) = self.pending_transfer_pieces.get_mut(&piece) {
                    work.failed = true;
                    work.writes_in_flight > 0
                } else {
                    false
                };
            if !has_in_flight_writes {
                self.fail_piece_transfer(piece);
            }
            return;
        }
        let aggregate_piece_write = self.can_aggregate_piece_write(piece);
        if !aggregate_piece_write && self.transfer_jobs.len() >= MAX_TRANSFER_COMPLETION_JOBS {
            // The peer has already delivered the block, but retaining another
            // payload would exceed this torrent's storage-work bound. Return
            // both owners so the block can be requested again.
            if let Some(conn) = transfer_conn {
                self.transfer_ledger.on_reject(conn, transfer_block);
            }
            self.picker.cancel_request(piece as usize, offset);
            return;
        }
        if let Some(conn) = transfer_conn {
            match self.transfer_ledger.on_block(conn, transfer_block) {
                ReceiveOutcome::Accepted { actions, .. } => {
                    self.apply_transfer_actions(actions);
                    if let Some(peer) = peer {
                        if let Some(handle) = self.active_peers.get_mut(&peer) {
                            if let Some(delay) = response_delay {
                                handle.request_window.on_response_delay(delay);
                            }
                            handle
                                .request_window
                                .on_delivery(u64::from(block_length), accepted_at);
                        }
                    }
                }
                ReceiveOutcome::Duplicate | ReceiveOutcome::IgnoredForPieceAffinity => {
                    self.picker.cancel_request(piece as usize, offset);
                    return;
                }
                ReceiveOutcome::StaleConnection
                | ReceiveOutcome::Unsolicited
                | ReceiveOutcome::Invalid(_) => {
                    self.picker.cancel_request(piece as usize, offset);
                    return;
                }
            }
        }
        if aggregate_piece_write {
            if let Err(e) = self.record_piece_block(&event.block).await {
                warn!(
                    component = "torrent",
                    operation = "assemble_piece",
                    torrent = %self.info_hash_hex,
                    piece,
                    offset,
                    result = "error",
                    error = %e,
                    "failed to assemble in-memory piece for verification"
                );
                self.remove_piece_assembly(piece);
                self.fail_piece_transfer(piece);
                return;
            }
            self.record_download(event.network_bytes.unwrap_or(event.block.data.len() as u64))
                .await;
            let complete = self.picker.block_received_pending(piece as usize, offset);
            self.cancel_endgame_duplicates(BlockRequest {
                piece,
                begin: offset,
                length: event.block.data.len() as u32,
            });
            if complete {
                let work = self.ensure_pending_transfer_piece(piece);
                work.aggregate_write = true;
                work.verify_ready = true;
            } else {
                self.pending_transfer_pieces.remove(&piece);
            }
            self.start_ready_transfer_jobs();
            if !complete && !self.picker.is_complete() {
                self.persist_progress_throttled(false).await;
            }
            return;
        }

        let work = self.ensure_pending_transfer_piece(piece);
        let generation = work.generation;
        work.aggregate_write = false;
        work.writes_in_flight = work.writes_in_flight.saturating_add(1);
        self.pending_transfer_blocks
            .insert((piece, offset), generation);

        let length = event.block.data.len() as u64;
        let downloaded_bytes = event.network_bytes.unwrap_or(length);
        let context = self.transfer_io_context();
        self.transfer_jobs.push(Box::pin(async move {
            let result = write_transfer_block(&context, event)
                .await
                .map_err(|error| error.to_string());
            TransferCompletion::BlockWritten {
                piece,
                generation,
                offset,
                length,
                downloaded_bytes,
                result,
            }
        }));
    }

    pub(super) fn start_ready_transfer_jobs(&mut self) {
        if self.transfer_jobs.len() >= MAX_TRANSFER_COMPLETION_JOBS {
            return;
        }
        let ready_pieces: Vec<u32> = self
            .pending_transfer_pieces
            .iter()
            .filter_map(|(&piece, work)| {
                (work.writes_in_flight == 0
                    && !work.failed
                    && !work.verify_in_flight
                    && (work.verify_ready || work.aggregate_write_ready))
                    .then_some(piece)
            })
            .collect();
        let mut ready_pieces = ready_pieces;
        ready_pieces.sort_unstable();
        for piece in ready_pieces {
            if self.transfer_jobs.len() >= MAX_TRANSFER_COMPLETION_JOBS {
                break;
            }
            let Some(work) = self.pending_transfer_pieces.get(&piece) else {
                continue;
            };
            let writing_aggregate = work.aggregate_write_ready;
            let aggregate_piece = work.aggregate_write;
            let generation = work.generation;
            let verified_payload = work.verified_payload.clone();
            let assembled = if !writing_aggregate && aggregate_piece {
                self.piece_assemblies.get(&piece).and_then(|assembly| {
                    assembly
                        .completed_data
                        .as_ref()
                        .cloned()
                        .map(|data| (data, assembly._memory_lease.as_ref().map(Arc::clone)))
                })
            } else {
                None
            };
            let Some(work) = self.pending_transfer_pieces.get_mut(&piece) else {
                continue;
            };
            if !writing_aggregate && aggregate_piece && assembled.is_none() {
                // An assembly may have been spilled to disk while waiting for
                // another bounded job slot. Its bytes are now verified from
                // the payload files and need no second aggregate write.
                work.aggregate_write = false;
            }
            work.verify_ready = false;
            work.aggregate_write_ready = false;
            work.verify_in_flight = true;

            let context = self.transfer_io_context();
            if writing_aggregate {
                let Some((data, lease)) = verified_payload else {
                    self.fail_piece_transfer(piece);
                    continue;
                };
                self.transfer_jobs.push(Box::pin(async move {
                    let result = write_transfer_piece(&context, piece, data, lease)
                        .await
                        .map_err(|error| error.to_string());
                    TransferCompletion::AggregatePieceWritten {
                        piece,
                        generation,
                        result,
                    }
                }));
            } else {
                let hybrid_v2 = self.hybrid_v2.clone();
                if assembled.is_some() {
                    self.completed_piece_verify_from_memory =
                        self.completed_piece_verify_from_memory.saturating_add(1);
                } else {
                    self.completed_piece_verify_from_disk =
                        self.completed_piece_verify_from_disk.saturating_add(1);
                }
                self.transfer_jobs.push(Box::pin(async move {
                    let result = verify_transfer_piece(
                        context,
                        hybrid_v2,
                        piece,
                        assembled
                            .clone()
                            .map(|(data, lease)| (data.clone(), lease.clone())),
                    )
                    .await;
                    TransferCompletion::PieceVerified {
                        piece,
                        generation,
                        result,
                        assembled,
                    }
                }));
            }
        }
    }

    pub(super) async fn handle_transfer_completion(&mut self, completion: TransferCompletion) {
        match completion {
            TransferCompletion::BlockWritten {
                piece,
                generation,
                offset,
                length,
                downloaded_bytes,
                result,
            } => {
                let coordinate = (piece, offset);
                let block_generation = self.pending_transfer_blocks.get(&coordinate).copied();
                let current_generation = self
                    .pending_transfer_pieces
                    .get(&piece)
                    .map(|work| work.generation);
                if block_generation != Some(generation) || current_generation != Some(generation) {
                    if block_generation == Some(generation) {
                        self.pending_transfer_blocks.remove(&coordinate);
                    }
                    if result.is_ok() {
                        self.note_data_write();
                        self.record_download(downloaded_bytes).await;
                    }
                    debug!(
                        component = "storage",
                        operation = "apply_block_completion",
                        torrent = %self.info_hash_hex,
                        piece,
                        offset,
                        generation,
                        result = "ignored",
                        reason = "stale_transfer_generation",
                        "ignoring a block completion from a replaced piece attempt"
                    );
                    self.start_ready_transfer_jobs();
                    return;
                }
                self.pending_transfer_blocks.remove(&coordinate);
                let Some(work) = self.pending_transfer_pieces.get_mut(&piece) else {
                    return;
                };
                let succeeded = result.is_ok();
                let error = result.err();
                work.writes_in_flight = work.writes_in_flight.saturating_sub(1);
                if let Some(error) = error {
                    work.failed = true;
                    warn!(
                        component = "storage",
                        operation = "write_block",
                        torrent = %self.info_hash_hex,
                        piece,
                        offset,
                        result = "error",
                        error = %error,
                        "block write failed"
                    );
                }
                let accept_written_block = succeeded && !work.failed;
                let failed_after_drain = work.failed && work.writes_in_flight == 0;

                if succeeded {
                    self.note_data_write();
                    self.record_download(downloaded_bytes).await;
                }
                let mut complete_piece = false;
                if accept_written_block {
                    complete_piece = self.picker.block_received_pending(piece as usize, offset);
                    self.cancel_endgame_duplicates(BlockRequest {
                        piece,
                        begin: offset,
                        length: u32::try_from(length).unwrap_or(u32::MAX),
                    });
                    if complete_piece {
                        if let Some(work) = self.pending_transfer_pieces.get_mut(&piece) {
                            work.verify_ready = true;
                        }
                    }
                }

                let clear_partial_state =
                    self.pending_transfer_pieces
                        .get(&piece)
                        .is_some_and(|work| {
                            !work.failed
                                && work.writes_in_flight == 0
                                && !work.verify_ready
                                && !work.verify_in_flight
                                && !work.aggregate_write_ready
                        });
                if clear_partial_state && !complete_piece {
                    self.pending_transfer_pieces.remove(&piece);
                }

                if failed_after_drain {
                    self.fail_piece_transfer(piece);
                } else {
                    self.start_ready_transfer_jobs();
                    if !complete_piece && !self.picker.is_complete() {
                        self.persist_progress_throttled(false).await;
                    }
                }
            }
            TransferCompletion::PieceVerified {
                piece,
                generation,
                result,
                assembled,
            } => {
                if self
                    .pending_transfer_pieces
                    .get(&piece)
                    .map(|work| work.generation)
                    != Some(generation)
                {
                    debug!(
                        component = "storage",
                        operation = "apply_piece_verification",
                        torrent = %self.info_hash_hex,
                        piece,
                        generation,
                        result = "ignored",
                        reason = "stale_transfer_generation",
                        "ignoring verification from a replaced piece attempt"
                    );
                    self.start_ready_transfer_jobs();
                    return;
                }
                let Some(work) = self.pending_transfer_pieces.get_mut(&piece) else {
                    return;
                };
                work.verify_in_flight = false;
                let aggregate_piece = work.aggregate_write;
                match result {
                    VerifyResult::Valid if aggregate_piece => {
                        if let Some(work) = self.pending_transfer_pieces.get_mut(&piece) {
                            work.aggregate_write_ready = true;
                            work.verified_payload = assembled;
                        }
                        self.start_ready_transfer_jobs();
                    }
                    VerifyResult::Valid => self.finish_verified_piece(piece).await,
                    VerifyResult::Invalid => {
                        warn!(piece, torrent = %self.info_hash_hex, "piece verification failed");
                        self.fail_piece_transfer(piece);
                    }
                    VerifyResult::Missing { file_index, reason } => {
                        warn!(
                            piece,
                            file_index,
                            reason = %reason,
                            torrent = %self.info_hash_hex,
                            "piece verification could not read data"
                        );
                        self.fail_piece_transfer(piece);
                    }
                }
                if !self.picker.is_complete() {
                    self.persist_progress_throttled(false).await;
                }
            }
            TransferCompletion::AggregatePieceWritten {
                piece,
                generation,
                result,
            } => {
                if self
                    .pending_transfer_pieces
                    .get(&piece)
                    .map(|work| work.generation)
                    != Some(generation)
                {
                    if result.is_ok() {
                        self.note_data_write();
                    }
                    debug!(
                        component = "storage",
                        operation = "apply_aggregate_write_completion",
                        torrent = %self.info_hash_hex,
                        piece,
                        generation,
                        result = "ignored",
                        reason = "stale_transfer_generation",
                        "ignoring aggregate write completion from a replaced piece attempt"
                    );
                    self.start_ready_transfer_jobs();
                    return;
                }
                match result {
                    Ok(()) => {
                        self.note_data_write();
                        self.finish_verified_piece(piece).await;
                        if !self.picker.is_complete() {
                            self.persist_progress_throttled(false).await;
                        }
                    }
                    Err(error) => {
                        warn!(
                            component = "storage",
                            operation = "write_completed_piece",
                            torrent = %self.info_hash_hex,
                            piece,
                            result = "error",
                            error = %error,
                            "completed piece write failed"
                        );
                        self.fail_piece_transfer(piece);
                        self.persist_progress_throttled(false).await;
                    }
                }
            }
        }
        self.start_ready_transfer_jobs();
    }

    pub(super) fn fail_piece_transfer(&mut self, piece: u32) {
        let (_, actions) = self.transfer_ledger.piece_failed_with_actions(piece);
        self.apply_transfer_actions(actions);
        self.release_transfer_piece_memory(piece);
        self.picker.reject_piece(piece as usize);
        self.restored_partial_pieces.remove(&piece);
        self.remove_piece_assembly(piece);
        if let Some(work) = self.pending_transfer_pieces.remove(&piece) {
            self.pending_transfer_blocks
                .retain(|_, generation| *generation != work.generation);
        }
    }

    async fn finish_verified_piece(&mut self, piece: u32) {
        let actions = self.transfer_ledger.mark_have(piece);
        self.apply_transfer_actions(actions);
        self.release_transfer_piece_memory(piece);
        self.picker.mark_have(piece as usize);
        self.restored_partial_pieces.remove(&piece);
        self.remove_piece_assembly(piece);
        if let Some(work) = self.pending_transfer_pieces.remove(&piece) {
            self.pending_transfer_blocks
                .retain(|_, generation| *generation != work.generation);
        }
        self.dirty_pieces_since_barrier.insert(piece);
        info!(
            component = "torrent",
            operation = "complete_piece",
            torrent = %self.info_hash_hex,
            piece,
            result = "ok",
            "piece complete"
        );
        self.send_have_to_peers(piece).await;
        if self.picker.is_complete() {
            self.on_download_complete().await;
        }
    }

    pub(super) fn can_aggregate_piece_write(&self, piece: u32) -> bool {
        self.piece_length(piece)
            .map(|len| {
                len as usize <= self.piece_assembly_soft_cap_bytes
                    && !self.restored_partial_pieces.contains(&piece)
            })
            .unwrap_or(false)
    }

    pub(super) async fn record_piece_block(&mut self, block: &BlockEvent) -> anyhow::Result<()> {
        let len = self.piece_length(block.piece)? as usize;
        if len > self.piece_assembly_soft_cap_bytes {
            return Ok(());
        }

        let inserted = if self.piece_assemblies.contains_key(&block.piece) {
            false
        } else {
            let memory_lease = reserve_piece_assembly_bytes(&self.resources, len)?;
            self.piece_assembly_bytes = self.piece_assembly_bytes.saturating_add(len);
            self.piece_assemblies.insert(
                block.piece,
                PieceAssembly::with_memory_lease(len, memory_lease),
            );
            true
        };

        let result = self
            .piece_assemblies
            .get_mut(&block.piece)
            .expect("piece assembly inserted or already present")
            .insert(block.offset, &block.data);
        if result.is_err() && inserted {
            self.remove_piece_assembly(block.piece);
        }
        result?;
        self.enforce_piece_assembly_budget(block.piece).await;
        Ok(())
    }

    pub(super) fn remove_piece_assembly(&mut self, piece: u32) {
        if let Some(assembly) = self.piece_assemblies.remove(&piece) {
            self.piece_assembly_bytes = self.piece_assembly_bytes.saturating_sub(assembly.len());
        }
    }

    pub(super) fn clear_piece_assemblies(&mut self) {
        let discarded_pieces = self.piece_assemblies.keys().copied().collect::<Vec<_>>();
        self.piece_assemblies.clear();
        self.piece_assembly_bytes = 0;
        // In-memory assemblies contain the only copy of partial blocks when
        // aggregation is enabled. Any clear that is not preceded by a
        // successful fastresume flush must also release the picker's partial
        // reservations, otherwise a later peer can be asked only for the
        // missing suffix while the discarded prefix is no longer available.
        for piece in discarded_pieces {
            self.fail_piece_transfer(piece);
        }
    }

    pub(super) async fn enforce_piece_assembly_budget(&mut self, current_piece: u32) {
        let evicted = evict_piece_assemblies_to_budget(
            &mut self.piece_assemblies,
            &mut self.piece_assembly_bytes,
            current_piece,
            MAX_IN_MEMORY_PIECE_ASSEMBLIES,
            self.piece_assembly_soft_cap_bytes,
        );
        self.piece_assembly_evictions = self
            .piece_assembly_evictions
            .saturating_add(evicted.len() as u64);
        for (piece, assembly) in evicted {
            // The assembly is the only copy of these blocks. Re-requesting
            // them would spend network bytes, picker state and time to undo a
            // memory bound, so spill what was received into the payload files
            // through the same direct-write path restored partial pieces use.
            // The picker keeps its received state and the piece completes by
            // verifying from disk.
            let mut spilled = true;
            let received_blocks = match assembly.into_received_blocks(piece) {
                Ok(blocks) => blocks,
                Err(error) => {
                    warn!(
                        component = "storage",
                        operation = "spill_piece_assembly",
                        torrent = %self.info_hash_hex,
                        piece,
                        result = "error",
                        error = %error,
                        "could not reserve memory to spill an evicted piece assembly"
                    );
                    spilled = false;
                    Vec::new()
                }
            };
            for (offset, data) in received_blocks {
                if let Err(error) = self.write_leased_block(&data).await {
                    warn!(
                        component = "storage",
                        operation = "spill_piece_assembly",
                        torrent = %self.info_hash_hex,
                        piece,
                        offset,
                        result = "error",
                        error = %error,
                        "failed to spill an evicted piece assembly; re-requesting its blocks"
                    );
                    spilled = false;
                    break;
                }
            }
            if spilled {
                self.restored_partial_pieces.insert(piece);
            } else {
                // Dropping the assembly while leaving the picker's
                // received-bit state intact would make the next block
                // recreate an incomplete zero-filled assembly. Fall back to
                // requesting the affected blocks again.
                self.fail_piece_transfer(piece);
            }
        }
    }

    pub(super) async fn send_have_to_peers(&mut self, piece: u32) {
        if self.super_seeding && self.picker.is_complete() {
            return;
        }
        let peers: Vec<SocketAddr> = self.active_peers.keys().copied().collect();
        let mut queue_full = 0u64;
        for peer in peers {
            if let Some(handle) = self.active_peers.get_mut(&peer) {
                if !Self::try_send_have(&handle.cmd_tx, &mut handle.pending_have, piece) {
                    queue_full = queue_full.saturating_add(1);
                }
            }
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    pub(super) fn retry_pending_peer_controls(&mut self) {
        let mut queue_full = 0u64;
        for handle in self.active_peers.values_mut() {
            while let Some(piece) = handle.pending_have.first_set_u32() {
                match handle.cmd_tx.try_send(PeerCommand::Have(piece)) {
                    Ok(()) => handle.pending_have.set(piece as usize, false),
                    Err(_) => {
                        queue_full = queue_full.saturating_add(1);
                        break;
                    }
                }
            }
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    pub(super) fn try_send_have(
        tx: &mpsc::Sender<PeerCommand>,
        pending: &mut PieceBitmap,
        piece: u32,
    ) -> bool {
        match tx.try_send(PeerCommand::Have(piece)) {
            Ok(()) => {
                pending.set(piece as usize, false);
                true
            }
            Err(_) => {
                pending.set(piece as usize, true);
                false
            }
        }
    }

    pub(super) fn merge_unique_block_requests(
        mut requests: Vec<BlockRequest>,
        additional: Vec<BlockRequest>,
    ) -> Vec<BlockRequest> {
        let mut seen = requests
            .iter()
            .map(|request| (request.piece, request.begin))
            .collect::<HashSet<_>>();
        for request in additional {
            if seen.insert((request.piece, request.begin)) {
                requests.push(request);
            }
        }
        requests
    }

    pub(super) fn reconcile_fast_choke_requests(
        actor_requested: Vec<BlockRequest>,
        peer_outstanding: Vec<BlockRequest>,
    ) -> (Vec<BlockRequest>, Vec<BlockRequest>) {
        let peer_coordinates = peer_outstanding
            .iter()
            .map(|request| (request.piece, request.begin))
            .collect::<std::collections::HashSet<_>>();
        let actor_coordinates = actor_requested
            .iter()
            .map(|request| (request.piece, request.begin))
            .collect::<std::collections::HashSet<_>>();
        let mut on_wire = Vec::new();
        let mut cancel = Vec::new();
        for request in actor_requested {
            if peer_coordinates.contains(&(request.piece, request.begin)) {
                on_wire.push(request);
            } else {
                cancel.push(request);
            }
        }
        for request in peer_outstanding {
            if !actor_coordinates.contains(&(request.piece, request.begin)) {
                on_wire.push(request);
            }
        }
        (on_wire, cancel)
    }

    /// Ask every other peer that has this accepted block outstanding to stop
    /// sending a duplicate. The peer loop retains an on-wire response lease
    /// until Piece, Reject, timeout, or disconnect because Cancel can race a
    /// response already in flight.
    pub(super) fn cancel_endgame_duplicates(&mut self, accepted: BlockRequest) {
        let mut queue_full = 0u64;
        for handle in self.active_peers.values() {
            let owns_request = handle.requested.iter().any(|request| {
                request.piece == accepted.piece
                    && request.begin == accepted.begin
                    && request.length == accepted.length
            });
            if owns_request
                && handle
                    .cmd_tx
                    .try_send(PeerCommand::Cancel { request: accepted })
                    .is_err()
            {
                queue_full = queue_full.saturating_add(1);
            }
        }
        self.peer_command_queue_full = self.peer_command_queue_full.saturating_add(queue_full);
    }

    pub(super) fn download_tokens_available(&mut self, bytes: u32) -> bool {
        self.refill_download_tokens();
        if self.download_limit_bytes_per_sec.is_none() {
            return true;
        }
        self.download_tokens >= u64::from(bytes)
    }

    pub(super) fn try_consume_download_tokens(&mut self, bytes: u32) -> bool {
        self.refill_download_tokens();
        let Some(limit) = self.download_limit_bytes_per_sec else {
            return true;
        };
        consume_download_tokens(&mut self.download_tokens, limit, u64::from(bytes))
    }

    pub(super) fn refund_download_tokens(&mut self, bytes: u32) {
        self.refill_download_tokens();
        let Some(limit) = self.download_limit_bytes_per_sec else {
            return;
        };
        self.download_tokens = self
            .download_tokens
            .saturating_add(u64::from(bytes))
            .min(download_bucket_capacity(limit));
    }

    pub(super) fn refill_download_tokens(&mut self) {
        let now = Instant::now();
        let Some(limit) = self.download_limit_bytes_per_sec else {
            self.download_tokens = u64::MAX;
            self.download_tokens_updated = now;
            return;
        };
        let elapsed = now.saturating_duration_since(self.download_tokens_updated);
        self.download_tokens_updated = now;
        let refill = refilled_bytes(elapsed, limit);
        self.download_tokens = self
            .download_tokens
            .saturating_add(refill)
            .min(download_bucket_capacity(limit));
    }

    pub(super) async fn drain_transfer_jobs(&mut self) {
        while let Some(completion) = self.transfer_jobs.next().await {
            self.handle_transfer_completion(completion).await;
        }
    }

    pub(super) async fn shutdown_peers(&mut self) {
        self.close_webseed_transfer_connection();
        // Accepted transfer writes and hashes own their payload leases and
        // piece generation until completion. Drain them before resetting the
        // picker or changing file policy so no completion can cross that
        // boundary.
        self.drain_transfer_jobs().await;
        let handles: Vec<(
            mpsc::Sender<PeerCommand>,
            Option<tokio::task::AbortHandle>,
            RateLimitCancellation,
            RateLimitCancellation,
            ConnectionId,
        )> = self
            .active_peers
            .values()
            .map(|peer| {
                (
                    peer.cmd_tx.clone(),
                    peer.abort.clone(),
                    peer.upload_control.clone(),
                    peer.shutdown_control.clone(),
                    peer.transfer_conn,
                )
            })
            .collect();

        for peer in self.active_peers.values() {
            remove_peer_availability(&mut self.picker.availability, &peer.peer_has);
        }
        for (tx, abort, upload_control, shutdown_control, transfer_conn) in handles {
            self.transfer_ledger.disconnect(transfer_conn);
            upload_control.cancel();
            shutdown_control.cancel();
            let _ = tx.try_send(PeerCommand::Shutdown);
            if let Some(abort) = abort {
                abort.abort();
            }
        }
        self.active_peers.clear();
        self.picker.reset_outstanding_requests();
        self.clear_piece_assemblies();
        self.pending_transfer_blocks.clear();
        self.pending_transfer_pieces.clear();
        while self.peer_event_rx.try_recv().is_ok() {}
        while self.peer_disconnect_rx.try_recv().is_ok() {}
    }

    /// A ban update can race a full torrent command queue. Re-check the
    /// authoritative policy on a timer so an active connection is eventually
    /// evicted even if the best-effort control message was not enqueued.
    pub(super) async fn evict_banned_peers(&mut self) {
        if self.active_peers.is_empty() {
            return;
        }
        let registry = self.registry.read().await;
        let victims = self
            .active_peers
            .keys()
            .copied()
            .filter(|peer| registry.is_peer_banned(*peer))
            .collect::<Vec<_>>();
        drop(registry);
        for peer in victims {
            self.evict_peer(peer);
        }
    }

    pub(super) async fn record_download(&mut self, bytes: u64) {
        self.update_transfer(bytes, false).await;
    }

    pub(super) async fn record_upload(&mut self, bytes: u64) {
        self.last_upload_at = Instant::now();
        self.update_transfer(bytes, true).await;
        self.enforce_seed_limits().await;
    }

    pub(super) async fn update_transfer(&mut self, bytes: u64, upload: bool) {
        let mut reg = self.registry.write().await;
        let Some(mut entry) = reg.get_mut(&self.info_hash_hex) else {
            return;
        };
        if upload {
            entry.stats.add_upload(bytes);
        } else {
            entry.stats.add_download(bytes);
        }
        self.transfer_stats_dirty = true;
    }

    pub(super) async fn enforce_seed_limits(&mut self) {
        if self.paused || !self.picker.is_complete() {
            return;
        }
        let (uploaded, downloaded) = self.transfer_snapshot().await;
        let ratio_reached = self
            .seed_ratio_limit
            .is_some_and(|limit| downloaded > 0 && (uploaded as f64 / downloaded as f64) >= limit);
        let idle_reached = self.seed_idle_limit.is_some_and(|limit| {
            self.seeding_started_at
                .is_some_and(|started| started.elapsed() >= limit)
                && self.last_upload_at.elapsed() >= limit
        });
        if !(ratio_reached || idle_reached) {
            return;
        }
        let previous_recheck_restore_state = self.recheck_restore_state;
        self.paused = true;
        self.recheck_restore_state = Some(TorrentState::Paused);
        self.cancel_tracker_announces();
        self.announce_stopped_with_control_deadline().await;
        self.save_fastresume(false).await;
        self.shutdown_peers().await;
        match self.set_state_checked(TorrentState::Paused).await {
            Ok(()) => {
                self.tracker_event = TrackerEvent::Started;
            }
            Err(error) => {
                self.paused = false;
                self.recheck_restore_state = previous_recheck_restore_state;
                self.restart_tracker_session();
                warn!(
                    component = "torrent",
                    operation = "seed_limit",
                    torrent = %self.info_hash_hex,
                    result = "error",
                    error = %error,
                    "failed to persist seed-limit pause; retaining active state"
                );
                return;
            }
        }
        info!(
            component = "torrent",
            operation = "seed_limit",
            torrent = %self.info_hash_hex,
            ratio_reached,
            idle_reached,
            result = "paused",
            "torrent paused after reaching its seeding limit"
        );
    }

    pub(super) async fn transfer_snapshot(&self) -> (u64, u64) {
        let reg = self.registry.read().await;
        reg.get(&self.info_hash_hex)
            .map(|entry| (entry.stats.uploaded, entry.stats.downloaded))
            .unwrap_or((0, 0))
    }
}
