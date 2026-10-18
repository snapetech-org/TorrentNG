//! Accepted TCP/uTP peer sessions and bounded webseed request handling.
//! This module owns transport setup while the torrent task owns session state.

use crate::network_budget::token_refill_delay;

use super::*;

impl TorrentTask {
    pub(super) async fn accept_peer(
        &mut self,
        stream: TcpStream,
        peer_addr: SocketAddr,
        handshake: Handshake,
        peer_permit: OwnedSemaphorePermit,
    ) {
        if handshake.peer_id == crate::peer_id::our_peer_id() {
            debug!(
                component = "peer",
                operation = "accept_incoming",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "self_peer_id",
                "rejecting an incoming connection using this client peer id"
            );
            return;
        }
        if self.registry.read().await.is_peer_banned(peer_addr) {
            debug!(
                component = "peer",
                operation = "accept_incoming",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_banned",
                "rejecting banned incoming peer"
            );
            return;
        }
        if self.active_peers.len() >= self.peer_capacity()
            || self.active_peers.contains_key(&peer_addr)
        {
            return;
        }
        if !self.peer_source_allowed(peer_addr) {
            debug!(
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                "rejecting inbound peer not returned by private tracker"
            );
            return;
        }
        let info_hash = self.meta.info_hash;
        let Some((peer_id, peer_cmd_rx)) = self.register_peer(peer_addr, peer_permit) else {
            debug!(
                component = "peer",
                operation = "register_incoming",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_state_memory_budget",
                "peer state memory budget exhausted"
            );
            return;
        };
        let peer_event_tx = self.peer_event_tx.clone();
        let peer_disconnect_tx = self.peer_disconnect_tx.clone();
        let Some(upload) = self.upload_context(peer_addr) else {
            self.discard_registered_peer(peer_addr);
            debug!(
                component = "peer",
                operation = "allocate_incoming_state",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_state_memory_budget",
                "peer state memory budget exhausted"
            );
            return;
        };
        let peer_task = tokio::spawn(async move {
            let (result, outstanding) = match run_incoming_peer(
                stream,
                peer_addr,
                peer_id,
                info_hash,
                peer_event_tx,
                peer_cmd_rx,
                upload,
                handshake.reserved.supports_extension_protocol(),
                handshake.reserved.supports_fast_extension(),
            )
            .await
            {
                Ok(exit) => (exit.result, exit.outstanding),
                Err(error) => (Err(error), Vec::new()),
            };
            if let Err(e) = result {
                debug!(
                    component = "peer",
                    operation = "run_incoming",
                    peer = %peer_addr,
                    result = "ended",
                    error = %e,
                    "incoming peer ended"
                );
            }
            let _ = send_peer_disconnect_event(
                &peer_disconnect_tx,
                PeerEvent::Disconnected {
                    peer: peer_addr,
                    id: peer_id,
                    outstanding,
                },
            )
            .await;
        });
        self.attach_peer_abort(peer_addr, peer_task.abort_handle());
    }

    pub(super) async fn accept_utp_peer(
        &mut self,
        stream: UtpStream,
        peer_addr: SocketAddr,
        handshake: Handshake,
        peer_permit: OwnedSemaphorePermit,
    ) {
        if handshake.peer_id == crate::peer_id::our_peer_id() {
            debug!(
                component = "peer",
                operation = "accept_incoming_utp",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "self_peer_id",
                "rejecting an incoming uTP connection using this client peer id"
            );
            return;
        }
        if self.registry.read().await.is_peer_banned(peer_addr) {
            debug!(
                component = "peer",
                operation = "accept_incoming_utp",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_banned",
                "rejecting banned incoming uTP peer"
            );
            return;
        }
        if self.active_peers.len() >= self.peer_capacity()
            || self.active_peers.contains_key(&peer_addr)
        {
            return;
        }
        if !self.peer_source_allowed(peer_addr) {
            debug!(
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                "rejecting inbound uTP peer not returned by private tracker"
            );
            return;
        }
        let info_hash = self.meta.info_hash;
        let Some((peer_id, peer_cmd_rx)) = self.register_peer(peer_addr, peer_permit) else {
            debug!(
                component = "peer",
                operation = "register_incoming_utp",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_state_memory_budget",
                "peer state memory budget exhausted"
            );
            return;
        };
        let peer_event_tx = self.peer_event_tx.clone();
        let peer_disconnect_tx = self.peer_disconnect_tx.clone();
        let Some(upload) = self.upload_context(peer_addr) else {
            self.discard_registered_peer(peer_addr);
            debug!(
                component = "peer",
                operation = "allocate_incoming_utp_state",
                torrent = %self.info_hash_hex,
                peer = %peer_addr,
                result = "rejected",
                reason = "peer_state_memory_budget",
                "peer state memory budget exhausted"
            );
            return;
        };
        let peer_task = tokio::spawn(async move {
            let (result, outstanding) = match run_incoming_utp_peer(
                stream,
                peer_addr,
                peer_id,
                info_hash,
                peer_event_tx,
                peer_cmd_rx,
                upload,
                handshake.reserved.supports_extension_protocol(),
                handshake.reserved.supports_fast_extension(),
            )
            .await
            {
                Ok(exit) => (exit.result, exit.outstanding),
                Err(error) => (Err(error), Vec::new()),
            };
            if let Err(e) = result {
                debug!(
                    component = "peer",
                    operation = "run_incoming_utp",
                    peer = %peer_addr,
                    result = "ended",
                    error = %e,
                    "incoming uTP peer ended"
                );
            }
            let _ = send_peer_disconnect_event(
                &peer_disconnect_tx,
                PeerEvent::Disconnected {
                    peer: peer_addr,
                    id: peer_id,
                    outstanding,
                },
            )
            .await;
        });
        self.attach_peer_abort(peer_addr, peer_task.abort_handle());
    }

    pub(super) fn upload_context(&self, peer_addr: SocketAddr) -> Option<UploadContext> {
        let peer = self.active_peers.get(&peer_addr)?;
        let upload_control = peer.upload_control.clone();
        let shutdown_control = peer.shutdown_control.clone();
        let piece_count = self.picker.piece_count();
        // Reserve the upload-side dense availability map before constructing
        // it. A torrent with a large piece count must not make every peer
        // connection allocate a bitmap that the governor would reject.
        let bitmap_memory_lease = self.resources.try_acquire(
            MemoryClass::PeerBuffer,
            PieceBitmap::memory_bytes_for_len(piece_count),
        )?;
        let have_pieces = if self.super_seeding && self.picker.is_complete() {
            super_seed_visible_pieces(&self.picker, piece_count, peer_addr)
        } else {
            let mut have_pieces = PieceBitmap::new(piece_count);
            for piece in 0..piece_count {
                if self.picker.have_piece(piece) {
                    have_pieces.set(piece, true);
                }
            }
            have_pieces
        };
        Some(UploadContext {
            save_root: self.save_root.clone(),
            piece_map: self.piece_map.clone(),
            storage: self.storage.clone(),
            resources: self.resources.clone(),
            have_pieces,
            _bitmap_memory_lease: Some(bitmap_memory_lease),
            metadata: self.metadata.clone(),
            is_private: self.meta.private,
            pex_enabled: self.pex_enabled,
            torrent_upload: Arc::clone(&self.torrent_upload),
            fast_negotiated: false,
            upload_control,
            shutdown_control,
            global_download: self.network_budget.download(),
            global_upload: self.network_budget.upload(),
        })
    }

    pub(super) fn register_peer(
        &mut self,
        addr: SocketAddr,
        peer_permit: OwnedSemaphorePermit,
    ) -> Option<(PeerId, mpsc::Receiver<PeerCommand>)> {
        // Must hold a full adaptive request window plus control messages.
        let (cmd_tx, cmd_rx) = mpsc::channel(PEER_REQUEST_PIPELINE_ADAPTIVE_MAX * 2);
        let id = PeerId::new();
        // Both dense availability maps and the fixed transfer-ledger
        // connection state live for the peer lifetime. Acquire their
        // combined budget before allocating either map so a rejected peer
        // never creates a large temporary bitmap.
        let bitmap_bytes = PieceBitmap::memory_bytes_for_len(self.meta.pieces.len())
            .saturating_mul(2)
            .saturating_add(TransferLedger::<Arc<PieceMap>>::estimated_connection_state_bytes());
        let bitmap_memory_lease = self
            .resources
            .try_acquire(MemoryClass::PeerBuffer, bitmap_bytes)?;
        let transfer_conn = self
            .transfer_ledger
            .try_connect(PEER_REQUEST_PIPELINE_ADAPTIVE_MAX)?;
        let peer_has = PieceBitmap::new(self.meta.pieces.len());
        let pending_have = PieceBitmap::new(self.meta.pieces.len());
        self.active_peers.insert(
            addr,
            PeerHandle {
                id,
                transfer_conn,
                cmd_tx,
                upload_control: RateLimitCancellation::default(),
                shutdown_control: RateLimitCancellation::default(),
                abort: None,
                peer_has,
                choked: true,
                upload_choked: true,
                interested: false,
                downloaded: 0,
                uploaded: 0,
                download_rate: 0.0,
                upload_rate: 0.0,
                download_rate_window: 0,
                upload_rate_window: 0,
                download_rate_window_started: Instant::now(),
                upload_rate_window_started: Instant::now(),
                outstanding: 0,
                download_generation: 0,
                requested: Vec::new(),
                ut_metadata_id: None,
                ut_pex_id: None,
                metadata_size: None,
                _peer_permit: peer_permit,
                _bitmap_memory_lease: bitmap_memory_lease,
                pending_have,
            },
        );
        Some((id, cmd_rx))
    }

    pub(super) fn attach_peer_abort(&mut self, addr: SocketAddr, abort: tokio::task::AbortHandle) {
        if let Some(peer) = self.active_peers.get_mut(&addr) {
            peer.abort = Some(abort);
        } else {
            // The peer can finish before the engine processes its first
            // event. Do not leave the just-created task alive in that race.
            abort.abort();
        }
    }

    pub(super) fn peer_snapshots_limited(
        &self,
        max_entries: usize,
    ) -> Result<Vec<EnginePeerSnapshot>, String> {
        if max_entries > MAX_PEER_SNAPSHOT_ITEMS {
            return Err(format!(
                "requested peer snapshot limit {max_entries} exceeds {MAX_PEER_SNAPSHOT_ITEMS}"
            ));
        }
        if self.active_peers.len() > max_entries {
            return Err(format!(
                "torrent peer snapshot contains {} peers; maximum is {max_entries}",
                self.active_peers.len()
            ));
        }
        let mut snapshots = Vec::with_capacity(self.active_peers.len());
        self.active_peers
            .iter()
            .map(|(addr, peer)| {
                let pieces = peer.peer_has.count_ones();
                let pieces_total = peer.peer_has.len();
                let progress = if pieces_total == 0 {
                    0.0
                } else {
                    pieces as f64 / pieces_total as f64
                };
                EnginePeerSnapshot {
                    addr: *addr,
                    client: peer_client_label(peer),
                    choked: peer.choked,
                    upload_choked: peer.upload_choked,
                    interested: peer.interested,
                    pieces,
                    pieces_total,
                    progress,
                    download_rate: peer_rate(peer.download_rate, peer.download_rate_window_started),
                    upload_rate: peer_rate(peer.upload_rate, peer.upload_rate_window_started),
                    downloaded: peer.downloaded,
                    uploaded: peer.uploaded,
                }
            })
            .for_each(|snapshot| snapshots.push(snapshot));
        Ok(snapshots)
    }

    pub(super) fn webseed_snapshots(&self) -> Vec<EngineWebseedSnapshot> {
        let now = Instant::now();
        self.meta
            .webseeds
            .iter()
            .enumerate()
            .map(|(idx, url)| {
                let recent = self
                    .webseed_last_success
                    .get(idx)
                    .and_then(|instant| *instant)
                    .is_some_and(|instant| now.duration_since(instant) <= Duration::from_secs(10));
                EngineWebseedSnapshot {
                    url: url.clone(),
                    is_downloading: recent,
                    download_rate: if recent {
                        self.webseed_last_rates.get(idx).copied().unwrap_or(0)
                    } else {
                        0
                    },
                    failures: self.webseed_failures.get(idx).copied().unwrap_or(0),
                }
            })
            .collect()
    }

    pub(super) fn runtime_stats(&self) -> TorrentRuntimeStats {
        let outstanding_requests = self
            .active_peers
            .values()
            .map(|peer| peer.outstanding as u64)
            .sum::<u64>();
        let peer_command_queue_capacity = self
            .active_peers
            .values()
            .map(|peer| peer.cmd_tx.max_capacity() as u64)
            .sum::<u64>();
        let peer_command_queue_depth = self
            .active_peers
            .values()
            .map(|peer| {
                peer.cmd_tx
                    .max_capacity()
                    .saturating_sub(peer.cmd_tx.capacity()) as u64
            })
            .sum::<u64>();
        let peer_command_queue_bytes = self
            .active_peers
            .values()
            .map(|peer| {
                (peer.cmd_tx.max_capacity() as u64)
                    .saturating_mul(std::mem::size_of::<PeerCommand>() as u64)
                    // This gauge also includes the per-peer packed
                    // availability/control maps. They are retained for the
                    // lifetime of the command mailbox and are charged to
                    // the same peer-buffer memory class at registration.
                    .saturating_add(peer.peer_has.memory_bytes())
                    .saturating_add(peer.pending_have.memory_bytes())
                    // The peer task retains its upload-side map for the same
                    // lifetime; it is charged when upload_context is built.
                    .saturating_add(peer.peer_has.memory_bytes())
                    .saturating_add(
                        peer.requested
                            .capacity()
                            .saturating_mul(std::mem::size_of::<BlockRequest>())
                            as u64,
                    )
            })
            .sum::<u64>();
        let tracker_peer_cache_bytes = self
            ._tracker_peer_cache_memory_lease
            .as_ref()
            .map_or(0, MemoryLease::bytes);
        let (download_rate, upload_rate) = self
            .active_peers
            .values()
            .map(|peer| {
                (
                    peer_rate(peer.download_rate, peer.download_rate_window_started),
                    peer_rate(peer.upload_rate, peer.upload_rate_window_started),
                )
            })
            .fold(
                (0_i64, 0_i64),
                |(download, upload), (peer_download, peer_upload)| {
                    (
                        download.saturating_add(peer_download),
                        upload.saturating_add(peer_upload),
                    )
                },
            );
        TorrentRuntimeStats {
            connected_peers: self.active_peers.len() as u64,
            outstanding_requests,
            download_rate,
            upload_rate,
            fastresume_dirty_pieces: self.dirty_pieces_since_barrier.len(),
            completed_piece_verify_from_memory: self.completed_piece_verify_from_memory,
            completed_piece_verify_from_disk: self.completed_piece_verify_from_disk,
            piece_assembly_buffers: self.piece_assemblies.len() as u64,
            piece_assembly_bytes: self.piece_assembly_bytes as u64,
            piece_assembly_evictions: self.piece_assembly_evictions,
            peer_request_window_reductions: self.peer_request_window_reductions,
            peer_rx_buffer_bytes: outstanding_requests.saturating_mul(MAX_BLOCK_SIZE as u64),
            peer_tx_buffer_bytes: 0,
            peer_command_queue_depth,
            peer_command_queue_capacity,
            peer_command_queue_full: self.peer_command_queue_full,
            tracker_peer_cache_entries: self.known_tracker_peers.len() as u64,
            tracker_peer_cache_drops: self.tracker_peer_cache_drops,
            tracker_peer_cache_bytes,
            peer_command_queue_bytes,
            storage: self.storage.stats(),
        }
    }

    pub(super) fn remember_tracker_peers(&mut self, peers: &[SocketAddr]) {
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
            // prepare_tracker_peer_cache reserved the full capacity before
            // this task could receive peers, so this insert cannot trigger a
            // new allocation.
            self.known_tracker_peers.insert(peer);
        }
        self.tracker_peer_cache_drops = self.tracker_peer_cache_drops.saturating_add(dropped);
    }

    pub(super) async fn retry_known_tracker_peers(&mut self) {
        if self.picker.is_complete() {
            return;
        }
        if self.active_peers.is_empty() {
            self.schedule_peerless_reannounce();
        }
        if self.active_peers.len() >= self.peer_capacity() || self.known_tracker_peers.is_empty() {
            return;
        }
        info!(
            component = "peer",
            operation = "retry_known_peers",
            torrent = %self.info_hash_hex,
            known_peers = self.known_tracker_peers.len(),
            result = "scheduled",
            "retrying known peers"
        );
        let peers: Vec<SocketAddr> = self.known_tracker_peers.iter().copied().collect();
        self.connect_peers(peers, PeerSource::Tracker).await;
    }

    pub(super) fn start_webseed_block(&mut self) -> bool {
        if self.webseed_fetch.is_some()
            || self.paused
            || !self.active_peers.is_empty()
            || self.picker.is_complete()
            || self.meta.webseeds.is_empty()
        {
            return false;
        }

        let Some(request) = self.picker.pick_from_seed() else {
            self.picker.reset_outstanding_requests();
            debug!(
                component = "webseed",
                operation = "select_block",
                torrent = %self.info_hash_hex,
                reason = "no_requestable_block",
                result = "skipped",
                "webseed skipped: no requestable block"
            );
            return false;
        };
        let Some(ranges) = webseed_block_ranges(&self.meta, request) else {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            debug!(
                component = "webseed",
                operation = "select_block",
                torrent = %self.info_hash_hex,
                reason = "unsupported_range_layout",
                result = "skipped",
                "webseed skipped: block range is not representable as bounded file reads"
            );
            return false;
        };
        let expected_network_bytes = ranges
            .iter()
            .filter(|range| !range.file.pad)
            .map(|range| u32::try_from(range.length).unwrap_or(u32::MAX))
            .fold(0u32, u32::saturating_add);
        if expected_network_bytes == 0 {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            return false;
        }
        if !self.download_tokens_available(expected_network_bytes) {
            if let Some(limit) = self.download_limit_bytes_per_sec {
                let missing =
                    u64::from(expected_network_bytes).saturating_sub(self.download_tokens);
                if missing > 0 {
                    let deadline = Instant::now() + token_refill_delay(missing, limit);
                    self.download_token_wake = Some(
                        self.download_token_wake
                            .map_or(deadline, |current| current.min(deadline)),
                    );
                }
            }
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            return false;
        }

        let seed_count = self.meta.webseeds.len();
        let now = Instant::now();
        let mut selected = None;
        for attempt in 0..seed_count {
            let index = (self.webseed_next_index + attempt) % seed_count;
            if self
                .webseed_next_attempt
                .get(index)
                .and_then(|next| *next)
                .is_some_and(|deadline| now < deadline)
            {
                continue;
            }
            if self
                .webseed_failures
                .get(index)
                .copied()
                .is_some_and(|failures| failures == u8::MAX)
            {
                continue;
            }

            let mut prepared = Vec::with_capacity(ranges.len());
            let mut supported = true;
            for range in ranges.iter().filter(|range| !range.file.pad) {
                let Some(url) =
                    webseed_file_url(&self.meta, range.file, &self.meta.webseeds[index])
                else {
                    supported = false;
                    break;
                };
                prepared.push(PreparedWebseedRange {
                    url,
                    file_offset: range.file_offset,
                    block_offset: range.block_offset,
                    length: range.length,
                });
            }
            if !supported || prepared.is_empty() {
                if let Some(failures) = self.webseed_failures.get_mut(index) {
                    *failures = u8::MAX;
                }
                debug!(
                    torrent = %self.info_hash_hex,
                    webseed = %url_log_target(&self.meta.webseeds[index]),
                    "webseed skipped: unsupported url"
                );
                continue;
            }
            selected = Some((index, prepared));
            break;
        }

        let Some((seed_index, prepared_ranges)) = selected else {
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            return false;
        };
        let local_tokens_reserved = self.download_limit_bytes_per_sec.is_some();
        if !self.try_consume_download_tokens(expected_network_bytes) {
            if let Some(limit) = self.download_limit_bytes_per_sec {
                let missing =
                    u64::from(expected_network_bytes).saturating_sub(self.download_tokens);
                if missing > 0 {
                    let deadline = Instant::now() + token_refill_delay(missing, limit);
                    self.download_token_wake = Some(
                        self.download_token_wake
                            .map_or(deadline, |current| current.min(deadline)),
                    );
                }
            }
            self.picker
                .cancel_request(request.piece as usize, request.begin);
            return false;
        }
        let Some(connection) = self.reserve_webseed_block(request) else {
            self.refund_download_tokens(expected_network_bytes);
            return false;
        };
        if !self.commit_webseed_block(connection, request) {
            self.release_webseed_block(connection, request);
            self.close_webseed_transfer_connection();
            self.refund_download_tokens(expected_network_bytes);
            return false;
        }

        debug!(
            torrent = %self.info_hash_hex,
            webseed = %url_log_target(&self.meta.webseeds[seed_index]),
            piece = request.piece,
            offset = request.begin,
            length = request.length,
            "fetching webseed block"
        );

        let egress_policy = self.egress_policy;
        let http_timeout = self.http_timeout;
        let resources = self.resources.clone();
        let global_download = self.network_budget.download();
        let observed_network_bytes = Arc::new(AtomicU64::new(0));
        let worker_observed_network_bytes = Arc::clone(&observed_network_bytes);
        let task = tokio::spawn(async move {
            let fetch_started = TokioInstant::now();
            let event = Self::fetch_prepared_webseed_block(
                request,
                prepared_ranges,
                egress_policy,
                http_timeout,
                resources,
            )
            .await
            .map_err(|error| webseed_error_for_log(&error))?;
            let fetch_elapsed = fetch_started.elapsed();
            let network_bytes = event.network_bytes.unwrap_or(u64::from(request.length));
            worker_observed_network_bytes.store(network_bytes, Ordering::Release);
            if network_bytes > 0 {
                global_download.acquire(network_bytes).await;
            }
            Ok(WebseedFetchedBlock {
                event,
                fetch_elapsed,
            })
        });
        self.webseed_fetch = Some(WebseedFetchInFlight {
            request,
            connection,
            seed_index,
            reserved_local_bytes: expected_network_bytes,
            local_tokens_reserved,
            observed_network_bytes,
            task,
        });
        true
    }

    pub(super) fn finish_webseed_fetch(
        &mut self,
        fetch: WebseedFetchInFlight,
        result: Result<Result<WebseedFetchedBlock, String>, tokio::task::JoinError>,
    ) -> Option<WebseedFetchedBlock> {
        match result {
            Ok(Ok(fetched)) => {
                let network_bytes = fetched
                    .event
                    .network_bytes
                    .unwrap_or(u64::from(fetch.request.length));
                self.reconcile_webseed_local_tokens(
                    fetch.reserved_local_bytes,
                    fetch.local_tokens_reserved,
                    network_bytes,
                );

                let rate = (network_bytes as f64 / fetched.fetch_elapsed.as_secs_f64().max(0.001))
                    .round() as i64;
                let seed_count = self.meta.webseeds.len();
                if seed_count > 0 {
                    self.webseed_next_index = (fetch.seed_index + 1) % seed_count;
                }
                if let Some(failures) = self.webseed_failures.get_mut(fetch.seed_index) {
                    *failures = 0;
                }
                if let Some(next_attempt) = self.webseed_next_attempt.get_mut(fetch.seed_index) {
                    *next_attempt = None;
                }
                if let Some(last_rate) = self.webseed_last_rates.get_mut(fetch.seed_index) {
                    *last_rate = rate.max(0);
                }
                if let Some(last_success) = self.webseed_last_success.get_mut(fetch.seed_index) {
                    *last_success = Some(Instant::now());
                }
                Some(fetched)
            }
            Ok(Err(error)) => {
                self.record_webseed_failure(fetch.seed_index, fetch.request, error);
                self.reconcile_webseed_local_tokens(
                    fetch.reserved_local_bytes,
                    fetch.local_tokens_reserved,
                    fetch.observed_network_bytes.load(Ordering::Acquire),
                );
                self.release_webseed_block(fetch.connection, fetch.request);
                self.close_webseed_transfer_connection();
                None
            }
            Err(error) => {
                self.record_webseed_failure(
                    fetch.seed_index,
                    fetch.request,
                    format!("webseed worker failed: {error}"),
                );
                self.reconcile_webseed_local_tokens(
                    fetch.reserved_local_bytes,
                    fetch.local_tokens_reserved,
                    fetch.observed_network_bytes.load(Ordering::Acquire),
                );
                self.release_webseed_block(fetch.connection, fetch.request);
                self.close_webseed_transfer_connection();
                None
            }
        }
    }

    pub(super) fn reconcile_webseed_local_tokens(
        &mut self,
        reserved_bytes: u32,
        had_reservation: bool,
        observed_network_bytes: u64,
    ) {
        let observed_bytes = u32::try_from(observed_network_bytes).unwrap_or(u32::MAX);
        if had_reservation {
            if observed_bytes < reserved_bytes {
                self.refund_download_tokens(reserved_bytes - observed_bytes);
            } else if observed_bytes > reserved_bytes
                && !self.try_consume_download_tokens(observed_bytes - reserved_bytes)
            {
                // Exact per-file response lengths normally make the
                // reservation equal this count. Keep the bucket empty if a
                // future source shape reports more bytes.
                self.download_tokens = 0;
            }
        } else if observed_bytes > 0
            && self.download_limit_bytes_per_sec.is_some()
            && !self.try_consume_download_tokens(observed_bytes)
        {
            self.download_tokens = 0;
        }
    }

    fn record_webseed_failure(&mut self, index: usize, request: BlockRequest, error: String) {
        let failures = if let Some(failures) = self.webseed_failures.get_mut(index) {
            if error.contains("HTTP 404") || error.contains("HTTP 410") {
                *failures = (*failures).max(2);
            } else {
                *failures = failures.saturating_add(1);
            }
            *failures
        } else {
            1
        };
        if let Some(next_attempt) = self.webseed_next_attempt.get_mut(index) {
            *next_attempt = Some(Instant::now() + webseed_retry_delay(failures));
        }
        if let Some(webseed) = self.meta.webseeds.get(index) {
            warn!(
                component = "webseed",
                operation = "fetch_block",
                torrent = %self.info_hash_hex,
                webseed = %url_log_target(webseed),
                piece = request.piece,
                offset = request.begin,
                result = "error",
                error = %error,
                "webseed block fetch failed"
            );
        }
    }

    async fn fetch_prepared_webseed_block(
        request: BlockRequest,
        ranges: Vec<PreparedWebseedRange>,
        egress_policy: OutboundEgressPolicy,
        http_timeout: Duration,
        resources: ResourceGovernor,
    ) -> anyhow::Result<LeasedBlockEvent> {
        let user_agent = crate::peer_id::user_agent();
        let first_url = ranges
            .first()
            .map(|range| &range.url)
            .ok_or_else(|| anyhow::anyhow!("webseed request has no payload ranges"))?;
        let client = egress_policy
            .http_client(
                OutboundTargetKind::Webseed,
                first_url,
                http_timeout,
                &user_agent,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error))?;

        let payload_len = usize::try_from(request.length)?;
        let output_lease = reserve_webseed_body_bytes(&resources, request.length)?;
        let mut data = Vec::new();
        data.try_reserve_exact(payload_len)?;
        data.resize(payload_len, 0);
        let mut network_bytes = 0u64;

        for range in ranges {
            let (start, end) = webseed_file_byte_range(range.file_offset, range.length)?;
            let body_lease = reserve_webseed_body_bytes(&resources, u32::try_from(range.length)?)?;
            let response = client
                .get(range.url)
                .header(RANGE, format!("bytes={start}-{end}"))
                .send()
                .await
                .map_err(|error| anyhow::anyhow!(error.without_url()))?;
            validate_webseed_range_response(response.status(), response.headers(), start, end)?;
            let bytes = bounded_response_body(response, range.length)
                .await
                .map_err(|error| anyhow::anyhow!(error))?;
            if bytes.len() != range.length {
                anyhow::bail!(
                    "expected {} bytes, received {} bytes",
                    range.length,
                    bytes.len()
                );
            }
            let data_end = range
                .block_offset
                .checked_add(range.length)
                .ok_or_else(|| anyhow::anyhow!("webseed block slice overflow"))?;
            data.get_mut(range.block_offset..data_end)
                .ok_or_else(|| anyhow::anyhow!("webseed file range exceeds requested block"))?
                .copy_from_slice(&bytes);
            network_bytes = network_bytes
                .checked_add(u64::try_from(range.length)?)
                .ok_or_else(|| anyhow::anyhow!("webseed network byte count overflow"))?;
            drop(bytes);
            drop(body_lease);
        }

        Ok(LeasedBlockEvent::new(
            BlockEvent {
                piece: request.piece,
                offset: request.begin,
                data: data.into(),
            },
            output_lease,
        )?
        .with_network_bytes(network_bytes))
    }

    #[cfg(test)]
    pub(super) async fn download_next_webseed_block(&mut self) -> Option<LeasedBlockEvent> {
        if !self.start_webseed_block() {
            return None;
        }
        let mut fetch = self.webseed_fetch.take()?;
        let result = (&mut fetch.task).await;
        self.finish_webseed_fetch(fetch, result)
            .map(|fetched| fetched.event)
    }

    pub(super) fn schedule_peerless_reannounce(&mut self) {
        let now = Instant::now();
        if self
            .last_peerless_reannounce
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(120))
        {
            return;
        }
        self.last_peerless_reannounce = Some(now);
        self.tracker_event = TrackerEvent::Empty;
        self.schedule_active_tracker_tier_now();
        info!(
            torrent = %self.info_hash_hex,
            "scheduled tracker reannounce after losing all peers"
        );
    }

    pub(super) fn peer_source_allowed(&self, peer: SocketAddr) -> bool {
        private_peer_source_allowed(self.meta.private, &self.known_tracker_peers, peer)
    }
}
