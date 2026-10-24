//! Exclusive, journaled public recovery sessions. The original private metainfo,
//! labels and counters remain in place; no private infohash enters public discovery.
use super::*;
use rt_db::swarm_switch_row::{self as journal, SwarmSwitchRow};
use rt_metainfo::swarm::{
    public_counterpart_with_allocation_reservation, validate_swarm_payload, PublicVariant,
    MAX_SWARM_METAINFO_BYTES,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmVariant {
    #[default]
    RemovePrivate,
    PrivateZero,
    RemovePrivateAndSource,
    PrivateZeroWithoutSource,
    Supplied,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwarmPreview {
    pub original_hash: String,
    pub public_hash: String,
    pub public_v2_hash: Option<String>,
    pub piece_length: u64,
    pub piece_count: usize,
    pub files: usize,
    pub total_bytes: u64,
    pub variant: SwarmVariant,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwarmStatus {
    pub original_hash: Option<String>,
    pub public_hash: Option<String>,
    pub phase: String,
    pub can_switch: bool,
    pub can_restore: bool,
    pub dht_enabled: bool,
    pub active_state: String,
    pub verified_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct PreparedSwarm {
    pub preview: SwarmPreview,
    pub public: TorrentMeta,
    pub original_files: Vec<rt_db::TorrentFileRow>,
    pub _memory: MemoryLease,
    pub _guard: EngineAddTaskGuard,
}

impl EngineHandle {
    pub async fn swarm_status(&self, info_hash: String) -> CmdResult<SwarmStatus> {
        let info_hash = canonical_info_hash_checked(info_hash)?;
        let (reply, rx) = oneshot::channel();
        self.send_command(EngineCmd::SwarmStatus { info_hash, reply })
            .await?;
        await_engine_reply(rx).await
    }

    async fn prepare_swarm(
        &self,
        info_hash: String,
        variant: SwarmVariant,
        supplied: Option<Vec<u8>>,
    ) -> CmdResult<PreparedSwarm> {
        let info_hash = canonical_info_hash_checked(info_hash)?;
        if supplied
            .as_ref()
            .is_some_and(|raw| raw.len() > MAX_SWARM_METAINFO_BYTES)
        {
            return Err("Swarm Switch metainfo exceeds 8 MiB".into());
        }
        let guard = try_acquire_engine_add_task()
            .ok_or("Swarm Switch preparation is busy; retry shortly")?;
        let raw = self
            .torrent_blob_limited(info_hash.clone(), MAX_SWARM_METAINFO_BYTES)
            .await?;
        let mut memory = reserve_torrent_parse_memory(&self.resources, raw.len(), "swarm switch")?;
        tokio::task::spawn_blocking(move || {
            let original =
                parse_torrent_with_memory_lease(&raw, &mut memory).map_err(|e| e.to_string())?;
            if meta_info_hash_hex(&original) != info_hash {
                return Err("Original metainfo identity does not match its session".into());
            }
            let v = match variant {
                SwarmVariant::RemovePrivate => Some(PublicVariant::RemovePrivate),
                SwarmVariant::PrivateZero => Some(PublicVariant::PrivateZero),
                SwarmVariant::RemovePrivateAndSource => Some(PublicVariant::RemovePrivateAndSource),
                SwarmVariant::PrivateZeroWithoutSource => {
                    Some(PublicVariant::PrivateZeroWithoutSource)
                }
                SwarmVariant::Supplied => None,
            };
            if v.is_some() && supplied.is_some() {
                return Err("Choose either a generated variant or a supplied torrent".into());
            }
            let input = if v.is_none() {
                supplied.as_deref().ok_or("Select a public .torrent file")?
            } else {
                &raw
            };
            let public = public_counterpart_with_allocation_reservation(input, v, &mut |bytes| {
                memory.try_grow(bytes as u64)
            })?;
            validate_swarm_payload(&original, &public)?;
            let files = meta_file_rows(&info_hash, &original);
            let (piece_length, piece_count) = match &public {
                TorrentMeta::V1(m) => (m.piece_length, m.pieces.len()),
                TorrentMeta::Hybrid(m, _) => (m.piece_length, m.pieces.len()),
                TorrentMeta::V2(m) => (m.piece_length, m.piece_count() as usize),
            };
            let preview = SwarmPreview {
                original_hash: info_hash,
                public_hash: meta_info_hash_hex(&public),
                public_v2_hash: public.v2_info_hash().map(hex::encode),
                piece_length,
                piece_count,
                files: files.len(),
                total_bytes: meta_total_length(&public),
                variant,
            };
            Ok(PreparedSwarm {
                preview,
                public,
                original_files: files,
                _memory: memory,
                _guard: guard,
            })
        })
        .await
        .map_err(|e| crate::task_join_error_summary("swarm preparation", &e))?
    }

    pub async fn preview_swarm(
        &self,
        info_hash: String,
        variant: SwarmVariant,
        supplied: Option<Vec<u8>>,
    ) -> CmdResult<super::swarm_probe::SwarmComparison> {
        let status = self.swarm_status(info_hash.clone()).await?;
        let (info_hash, variant, supplied) =
            if let (Some(original), Some(public)) = (status.original_hash, status.public_hash) {
                (
                    original,
                    SwarmVariant::Supplied,
                    Some(
                        self.torrent_blob_limited(public, MAX_SWARM_METAINFO_BYTES)
                            .await?,
                    ),
                )
            } else {
                (info_hash, variant, supplied)
            };
        let candidate = self
            .prepare_swarm(info_hash.clone(), variant, supplied)
            .await?
            .preview;
        let (reply, rx) = oneshot::channel();
        self.send_command(EngineCmd::SwarmProbeContext { info_hash, reply })
            .await?;
        let context = await_engine_reply(rx).await?;
        super::swarm_probe::compare(candidate, context, self.resources.clone()).await
    }

    pub async fn switch_to_public(
        &self,
        info_hash: String,
        variant: SwarmVariant,
        supplied: Option<Vec<u8>>,
        expected_public_hash: String,
    ) -> CmdResult<SwarmStatus> {
        let prepared = self.prepare_swarm(info_hash, variant, supplied).await?;
        if prepared.preview.public_hash != expected_public_hash {
            return Err("Candidate changed; compare swarms again before switching".into());
        }
        let (reply, rx) = oneshot::channel();
        self.send_command(EngineCmd::SwitchToPublic {
            prepared: Box::new(prepared),
            reply,
        })
        .await?;
        await_engine_reply(rx).await
    }

    pub async fn restore_private_swarm(&self, info_hash: String) -> CmdResult<SwarmStatus> {
        let info_hash = canonical_info_hash_checked(info_hash)?;
        let (reply, rx) = oneshot::channel();
        self.send_command(EngineCmd::RestorePrivateSwarm { info_hash, reply })
            .await?;
        await_engine_reply(rx).await
    }
}

impl Engine {
    pub(super) fn swarm_pair(&self, hash: &str) -> Option<&SwarmSwitchRow> {
        self.swarm_switches.get(hash)
    }

    pub(super) fn ensure_swarm_active(&self, hash: &str) -> CmdResult<()> {
        if let Some(pair) = self.swarm_pair(hash) {
            if pair.phase != "public" || pair.original_hash == hash {
                return Err(
                    "This torrent is parked by Swarm Switch; use Restore private swarm".into(),
                );
            }
        }
        Ok(())
    }

    pub(super) fn ensure_swarm_unlinked(&self, hash: &str) -> CmdResult<()> {
        if self.swarm_pair(hash).is_some() {
            return Err("Restore the private swarm before changing shared files, trackers, or removing payload data".into());
        }
        Ok(())
    }

    fn remember_swarm(&mut self, row: SwarmSwitchRow) {
        self.swarm_switches
            .insert(row.original_hash.clone(), row.clone());
        self.swarm_switches.insert(row.public_hash.clone(), row);
    }

    pub(super) async fn recover_swarm_switches(&mut self) -> CmdResult<()> {
        let rows = self
            .db_executor()
            .run_durable("recover_swarm_switches", |db| {
                journal::recover(db).map_err(|e| e.to_string())
            })
            .await?;
        for row in rows {
            self.remember_swarm(row);
        }
        Ok(())
    }

    pub(super) async fn swarm_status_inner(&self, hash: &str) -> CmdResult<SwarmStatus> {
        let pair = self.swarm_pair(hash);
        let h = hash.to_owned();
        let private = self
            .run_db("swarm_privacy", move |db| {
                rt_db::torrent_is_private(db, &h).map_err(|e| e.to_string())
            })
            .await?;
        let (original, public, phase) = match pair {
            Some(p) => (
                Some(p.original_hash.clone()),
                Some(p.public_hash.clone()),
                p.phase.clone(),
            ),
            None => (
                private.then(|| hash.to_owned()),
                None,
                if private {
                    "private"
                } else {
                    "public_independent"
                }
                .to_owned(),
            ),
        };
        let active = {
            let reg = self.registry.read().await;
            public
                .as_deref()
                .and_then(|h| reg.get(h))
                .or_else(|| reg.get(hash))
                .ok_or("Torrent not found")?
        };
        let can_switch = pair.is_none() && private && active.state != TorrentState::MetadataPending;
        Ok(SwarmStatus {
            original_hash: original,
            public_hash: public,
            phase,
            can_switch,
            can_restore: pair.is_some(),
            dht_enabled: self.swarm_dht_enabled().await?,
            active_state: active.state.as_str().to_owned(),
            verified_bytes: active.total_length.saturating_sub(active.amount_left),
            total_bytes: active.total_length,
        })
    }

    async fn swarm_dht_enabled(&self) -> CmdResult<bool> {
        let running = self
            .services
            .dht_tx
            .as_ref()
            .is_some_and(|tx| !tx.is_closed());
        let default = self.config.dht.enabled;
        self.run_db("swarm_dht_enabled", move |db| {
            Ok(running && setting_bool_with_default_checked(db, SETTING_NETWORK_DHT, default)?)
        })
        .await
    }

    async fn persist_swarm_phase(&mut self, row: &SwarmSwitchRow, phase: &str) -> CmdResult<()> {
        let original = row.original_hash.clone();
        let phase_owned = phase.to_owned();
        self.db_executor()
            .run_durable("swarm_phase", move |db| {
                journal::set_phase(db, &original, &phase_owned, unix_now_i64())
                    .map_err(|e| e.to_string())
            })
            .await?;
        let mut next = row.clone();
        next.phase = phase.to_owned();
        next.updated_at = unix_now_i64();
        self.remember_swarm(next);
        Ok(())
    }

    async fn park_swarm_actor(&mut self, hash: &str) -> CmdResult<()> {
        self.cancel_pending_torrent_promotion(
            hash,
            "Swarm Switch parked this torrent",
            JOB_STATE_CANCELLED,
        )
        .await;
        if let Some(tx) = self.runtime.torrent_chans.get(hash).cloned() {
            Self::quiesce_torrent_actor_for_cutover(&tx).await?;
            self.stop_torrent_actor_for_cutover(hash).await?;
        }
        self.unregister_dht_torrent(hash).await;
        if self
            .registry
            .read()
            .await
            .get(hash)
            .is_some_and(|e| e.state == TorrentState::Error)
        {
            self.set_registry_state(hash, TorrentState::Checking, None)
                .await?;
        }
        self.set_registry_state(hash, TorrentState::Paused, None)
            .await?;
        Ok(())
    }

    async fn start_swarm_checked(&mut self, hash: &str) -> CmdResult<()> {
        // Both supported actors honor Checking even if an older resume exists.
        self.set_registry_state(hash, TorrentState::Checking, None)
            .await?;
        let owner = self.ensure_transfer_owner_record(hash).await?.owner;
        let (_tx, dht, private) = self
            .spawn_transfer_actor_from_storage(hash, owner, false, TorrentState::Checking)
            .await?;
        if !private {
            self.register_dht_torrent(dht, hash).await;
        }
        Ok(())
    }

    pub(super) async fn switch_to_public_inner(
        &mut self,
        prepared: PreparedSwarm,
    ) -> CmdResult<SwarmStatus> {
        let original = &prepared.preview.original_hash;
        let public_hash = &prepared.preview.public_hash;
        if let Some(pair) = self.swarm_pair(original) {
            if pair.public_hash == *public_hash && pair.phase == "public" {
                return self.swarm_status_inner(original).await;
            }
            return Err(
                "A public session already exists; restore it before choosing another candidate"
                    .into(),
            );
        }
        if !self.swarm_dht_enabled().await? {
            return Err("Enable DHT in session settings before using a public swarm".into());
        }
        self.ensure_torrent_jobs_idle(original).await?;
        if self.registry.read().await.get(public_hash).is_some()
            || self.runtime.pending_torrent_adds.contains(public_hash)
            || self.swarm_pair(public_hash).is_some()
        {
            return Err("The public candidate is already loaded or reserved; remove that torrent without deleting files first".into());
        }
        let hash = original.clone();
        let (files, limits) = self
            .run_db("swarm_source_files", move |db| {
                Ok((
                    rt_db::list_torrent_files(db, &hash).map_err(|e| e.to_string())?,
                    match rt_db::get_torrent_limits(db, &hash) {
                        Ok(row) => Some(row),
                        Err(rt_db::DbError::NotFound(_)) => None,
                        Err(e) => return Err(e.to_string()),
                    },
                ))
            })
            .await?;
        if files.len() != prepared.original_files.len()
            || files
                .iter()
                .zip(&prepared.original_files)
                .any(|(a, b)| a.path != b.path || a.length != b.length || a.offset != b.offset)
        {
            return Err(
                "Files were renamed or remapped; restore the original layout before switching"
                    .into(),
            );
        }
        let entry = self
            .registry
            .read()
            .await
            .get(original)
            .ok_or("Original private torrent not found")?;
        let row = SwarmSwitchRow {
            original_hash: original.clone(),
            public_hash: public_hash.clone(),
            phase: "preparing".into(),
            updated_at: unix_now_i64(),
        };
        let intent = row.clone();
        // Install the in-memory fence before a durable write whose reply may
        // time out after SQLite commits. Never admit a second writer on an
        // ambiguous result. A successful ordered read can release the fence.
        self.remember_swarm(row.clone());
        if let Err(error) = self
            .db_executor()
            .run_durable("begin_swarm_switch", move |db| {
                journal::begin(db, &intent).map_err(|e| e.to_string())
            })
            .await
        {
            match self
                .run_db("inspect_swarm_intent", |db| {
                    journal::list(db).map_err(|e| e.to_string())
                })
                .await
            {
                Ok(rows) if !rows.iter().any(|r| r.original_hash == *original) => {
                    self.swarm_switches.remove(original);
                    self.swarm_switches.remove(public_hash);
                }
                _ => {
                    let _ = self.park_swarm_actor(original).await;
                }
            }
            return Err(error);
        }
        // Errors after this point retain an addressable recovery record. Neither
        // a timeout nor a failed disk operation silently resumes the private swarm.
        self.park_swarm_actor(original).await?;
        let config = Arc::clone(&self.config);
        let raw = meta_raw(&prepared.public).to_vec();
        let hash = public_hash.clone();
        tokio::task::spawn_blocking(move || {
            save_torrent_blob_from_config(&config, &hash, &raw).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| crate::task_join_error_summary("public metainfo save", &e))??;
        self.add_torrent_after_blob(
            prepared.public,
            Some(PathBuf::from(&entry.save_path)),
            true,
            entry.category.clone(),
            entry.tags.clone(),
            None,
        )
        .await?;
        // Stop a staged paused actor before applying file selection or limits.
        self.park_swarm_actor(public_hash).await?;
        let hash = public_hash.clone();
        let display_name = format!("{} [public swarm]", entry.name);
        let db_display_name = display_name.clone();
        self.db_executor().run_durable("copy_swarm_selection", move |db| {
            let tx = db.transaction().map_err(|e| e.to_string())?;
            tx.execute("UPDATE torrents SET name=?1 WHERE info_hash=?2", rusqlite::params![db_display_name, hash]).map_err(|e| e.to_string())?;
            for file in files {
                tx.execute("UPDATE torrent_files SET priority=?1, wanted=?2 WHERE info_hash=?3 AND file_index=?4", rusqlite::params![file.priority, file.wanted, hash, file.file_index]).map_err(|e| e.to_string())?;
            }
            if let Some(mut limits) = limits { limits.info_hash = hash; rt_db::upsert_torrent_limits_in_tx(&tx, &limits).map_err(|e| e.to_string())?; }
            tx.commit().map_err(|e| e.to_string())
        }).await?;
        if let Some(mut entry) = self.registry.write().await.get_mut(public_hash) {
            entry.name = display_name;
        }
        self.persist_swarm_phase(&row, "public").await?;
        self.start_swarm_checked(public_hash).await?;
        self.append_session_event(
            Some(original),
            "swarm_switched_public",
            Some("Public recovery session started"),
            serde_json::json!({"public_hash": public_hash}),
        );
        self.swarm_status_inner(original).await
    }

    pub(super) async fn discard_private_copy(
        &mut self,
        pair: SwarmSwitchRow,
    ) -> CmdResult<Option<String>> {
        if pair.phase != "public" {
            return Err(
                "Finish or restore the interrupted switch before deleting the private copy".into(),
            );
        }
        self.ensure_torrent_durable_job_idle(&pair.original_hash)
            .await?;
        self.park_swarm_actor(&pair.original_hash).await?;
        let original = pair.original_hash.clone();
        self.db_executor()
            .run_durable("discard_private_copy", move |db| {
                let tx = db.transaction().map_err(|e| e.to_string())?;
                tx.execute(
                    "DELETE FROM swarm_switches WHERE original_hash=?1",
                    [&original],
                )
                .map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM torrents WHERE info_hash=?1", [&original])
                    .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())
            })
            .await?;
        self.swarm_switches.remove(&pair.original_hash);
        self.swarm_switches.remove(&pair.public_hash);
        let _ = self.registry.write().await.remove(&pair.original_hash);
        self.runtime.tier_controller.remove(&pair.original_hash);
        self.runtime.tier_last_active.remove(&pair.original_hash);
        self.cleanup_swarm_metadata(&pair.original_hash).await;
        self.append_session_event(
            Some(&pair.public_hash),
            "swarm_private_copy_deleted",
            Some("Public torrent is independent; private restore is unavailable"),
            serde_json::json!({}),
        );
        Ok(None)
    }

    async fn cleanup_swarm_metadata(&self, hash: &str) {
        let config = Arc::clone(&self.config);
        let hash = hash.to_owned();
        let result = tokio::task::spawn_blocking(move || -> CmdResult<()> {
            FastresumeStore::new(fastresume_dir(&config))
                .delete(&hash)
                .map_err(|e| e.to_string())?;
            match rt_storage::remove_file_no_follow(&torrent_blob_path(&config, &hash)) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            warn!(
                component = "swarm_switch",
                "Unused metainfo cleanup failed; payload files were retained"
            );
        }
    }

    pub(super) async fn restore_private_swarm_inner(
        &mut self,
        hash: &str,
    ) -> CmdResult<SwarmStatus> {
        let Some(pair) = self.swarm_pair(hash).cloned() else {
            let status = self.swarm_status_inner(hash).await?;
            return if status.can_switch {
                Ok(status)
            } else {
                Err("No retained private torrent exists for this public torrent".into())
            };
        };
        // Check the actual retained artifact before retiring the public session.
        // A missing/corrupt private file must not destroy the remaining identity.
        let config = Arc::clone(&self.config);
        let resources = self.services.resources.clone();
        let original = pair.original_hash.clone();
        tokio::task::spawn_blocking(move || -> CmdResult<()> {
            let raw = load_torrent_blob_from_config_limited(&config, &original, MAX_SWARM_METAINFO_BYTES)
                .map_err(|_| "The retained private torrent file is unavailable; keep the public session or delete the retained private entry".to_owned())?;
            let mut memory = reserve_torrent_parse_memory(&resources, raw.len(), "private restore")?;
            let meta = parse_torrent_with_memory_lease(&raw, &mut memory).map_err(|_| "The retained private torrent file is invalid".to_owned())?;
            if !meta.is_private() || meta_info_hash_hex(&meta) != original {
                return Err("The retained private torrent identity does not match this session".into());
            }
            Ok(())
        }).await.map_err(|e| crate::task_join_error_summary("private restore validation", &e))??;
        // Check jobs without the inactive-swarm admission guard used by ordinary commands.
        for h in [&pair.original_hash, &pair.public_hash] {
            self.ensure_torrent_durable_job_idle(h).await?;
        }
        self.persist_swarm_phase(&pair, "restoring").await?;
        if self.registry.read().await.get(&pair.public_hash).is_some() {
            self.park_swarm_actor(&pair.public_hash).await?;
        }
        self.park_swarm_actor(&pair.original_hash).await?;
        let config = Arc::clone(&self.config);
        let original = pair.original_hash.clone();
        tokio::task::spawn_blocking(move || {
            FastresumeStore::new(fastresume_dir(&config))
                .delete(&original)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| crate::task_join_error_summary("private resume invalidation", &e))??;
        let original = pair.original_hash.clone();
        let public = pair.public_hash.clone();
        self.db_executor()
            .run_durable("finish_swarm_switch", move |db| {
                journal::finish(db, &original, &public).map_err(|e| e.to_string())
            })
            .await?;
        self.swarm_switches.remove(&pair.original_hash);
        self.swarm_switches.remove(&pair.public_hash);
        let _ = self.registry.write().await.remove(&pair.public_hash);
        self.runtime.tier_controller.remove(&pair.public_hash);
        self.runtime.tier_last_active.remove(&pair.public_hash);
        self.cleanup_swarm_metadata(&pair.public_hash).await;
        self.start_swarm_checked(&pair.original_hash).await?;
        self.append_session_event(
            Some(&pair.original_hash),
            "swarm_restored_private",
            Some("Private swarm restored with fresh verification"),
            serde_json::json!({"public_hash": pair.public_hash}),
        );
        self.swarm_status_inner(&pair.original_hash).await
    }
}
