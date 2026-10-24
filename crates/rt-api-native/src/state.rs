use rt_api_model::{
    ApiRuntimeMetrics, ChunkedBitSet, ChunkedVec, IdempotencyStore, TorrentSummary,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex as StdMutex;
use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    net::IpAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};

use rt_engine::EngineHandle;
#[cfg(test)]
use rt_session::RegistryChange;
use rt_session::{SessionRegistry, REGISTRY_DELTA_PAGE_MAX};

pub type JsonMap = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthCredentials {
    pub username: String,
    pub password: String,
}

const TORRENT_SNAPSHOT_CACHE_SIZE: usize = 4;
const TORRENT_SNAPSHOT_MAX_AGE: Duration = Duration::from_millis(750);
const LOGIN_ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
const MAX_LOGIN_ATTEMPTS: u8 = 10;
const MAX_TRACKED_LOGIN_CLIENTS: usize = 4_096;
type TorrentOrderCache = StdMutex<HashMap<&'static str, Arc<Vec<usize>>>>;

#[derive(Clone, Default)]
pub struct LoginAttemptLimiter {
    state: Arc<Mutex<LoginAttemptState>>,
}

#[derive(Default)]
struct LoginAttemptState {
    clients: HashMap<Option<IpAddr>, LoginAttemptWindow>,
}

struct LoginAttemptWindow {
    started: Instant,
    attempts: u8,
}

impl LoginAttemptLimiter {
    pub(crate) async fn begin_attempt(&self, client: Option<IpAddr>) -> Result<(), u64> {
        let now = Instant::now();
        let mut state = self.state.lock().await;
        if let Some(window) = state.clients.get_mut(&client) {
            let elapsed = now.saturating_duration_since(window.started);
            if elapsed >= LOGIN_ATTEMPT_WINDOW {
                *window = LoginAttemptWindow {
                    started: now,
                    attempts: 1,
                };
                return Ok(());
            }
            if window.attempts >= MAX_LOGIN_ATTEMPTS {
                let remaining = LOGIN_ATTEMPT_WINDOW.saturating_sub(elapsed);
                let seconds = remaining
                    .as_secs()
                    .saturating_add(u64::from(remaining.subsec_nanos() != 0))
                    .max(1);
                return Err(seconds);
            }
            window.attempts += 1;
            return Ok(());
        }

        if state.clients.len() >= MAX_TRACKED_LOGIN_CLIENTS {
            state.clients.retain(|_, window| {
                now.saturating_duration_since(window.started) < LOGIN_ATTEMPT_WINDOW
            });
            if state.clients.len() >= MAX_TRACKED_LOGIN_CLIENTS {
                if let Some(oldest) = state
                    .clients
                    .iter()
                    .min_by_key(|(_, window)| window.started)
                    .map(|(client, _)| *client)
                {
                    state.clients.remove(&oldest);
                }
            }
        }
        state.clients.insert(
            client,
            LoginAttemptWindow {
                started: now,
                attempts: 1,
            },
        );
        Ok(())
    }

    pub(crate) async fn clear(&self, client: Option<IpAddr>) {
        self.state.lock().await.clients.remove(&client);
    }
}

fn lock_order_cache(
    cache: &TorrentOrderCache,
) -> std::sync::MutexGuard<'_, HashMap<&'static str, Arc<Vec<usize>>>> {
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Sort orders are derived from the immutable snapshot. If a
            // previous panic interrupted a cache update, discard the partial
            // projection and let the next request rebuild it.
            let mut guard = poisoned.into_inner();
            guard.clear();
            cache.clear_poison();
            guard
        }
    }
}

#[derive(Debug, Clone)]
pub struct TorrentSnapshot {
    pub revision: u64,
    pub torrents: Arc<ChunkedVec<TorrentSnapshotItem>>,
    orders: Arc<TorrentOrderCache>,
    filters: Arc<TorrentFilterIndex>,
}

#[derive(Debug, Clone)]
pub struct TorrentSnapshotItem {
    pub summary: TorrentSummary,
    pub amount_left: u64,
    name_sort_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TorrentLabelFacet {
    pub name: String,
    pub count: usize,
    pub save_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentSnapshotError {
    Expired { revision: u64 },
}

#[derive(Clone)]
struct CachedTorrentSnapshot {
    snapshot: TorrentSnapshot,
    generated_at: Instant,
}

#[derive(Debug, Clone)]
struct TorrentFilterIndex {
    by_hash: Arc<HashMap<CompactHashKey, usize>>,
    by_state: HashMap<String, Arc<ChunkedBitSet>>,
    by_media_type: HashMap<String, Arc<ChunkedBitSet>>,
    by_category: HashMap<String, Arc<ChunkedBitSet>>,
    by_tag: HashMap<String, Arc<ChunkedBitSet>>,
    /// Inverted indexes for free-text name/hash filters. Queries still verify
    /// the exact substring in the handler, but a normal search no longer
    /// walks every torrent just to find candidates. The one- and two-byte
    /// indexes matter because clients routinely send short filters.
    by_text_byte: HashMap<u8, Arc<ChunkedBitSet>>,
    by_text_bigram: HashMap<[u8; 2], Arc<ChunkedBitSet>>,
    by_text_trigram: HashMap<[u8; 3], Arc<ChunkedBitSet>>,
}

/// Snapshot indexes keep canonical v1/v2 hashes in their wire-sized form.
/// Noncanonical test and placeholder keys retain their exact string semantics.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum CompactHashKey {
    V1([u8; 20]),
    V2([u8; 32]),
    Other(String),
}

impl CompactHashKey {
    fn from_hash(value: &str) -> Self {
        if let Some(hash) = decode_lower_hex::<20>(value) {
            return Self::V1(hash);
        }
        if let Some(hash) = decode_lower_hex::<32>(value) {
            return Self::V2(hash);
        }
        Self::Other(value.to_owned())
    }

    fn lookup_text<'a>(&'a self) -> Cow<'a, str> {
        match self {
            Self::V1(hash) => Cow::Owned(encode_hex(hash)),
            Self::V2(hash) => Cow::Owned(encode_hex(hash)),
            Self::Other(value) => Cow::Borrowed(value),
        }
    }
}

fn decode_lower_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    let encoded = value.as_bytes();
    if encoded.len() != N.checked_mul(2)? {
        return None;
    }
    let mut decoded = [0u8; N];
    for (index, pair) in encoded.as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = lower_hex_nibble(pair[0])?.checked_mul(16)? + lower_hex_nibble(pair[1])?;
    }
    Some(decoded)
}

fn lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// Shared application state threaded through axum handlers.
/// Cheap shared API state. Replacing a configuration field on a clone keeps
/// that replacement local; existing Arc-backed mutable stores remain shared.
#[derive(Clone)]
pub struct AppState(Arc<AppStateInner>);

impl std::ops::Deref for AppState {
    type Target = AppStateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for AppState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

/// Fields exposed through `AppState`; construct state with its constructors.
/// Copy-on-write clones configuration and retains the shared store handles.
#[derive(Clone)]
pub struct AppStateInner {
    pub registry: Arc<RwLock<SessionRegistry>>,
    pub engine: Option<EngineHandle>,
    pub api_tokens: Arc<Vec<String>>,
    pub auth_credentials: Arc<RwLock<AuthCredentials>>,
    pub configured_auth_credentials: AuthCredentials,
    pub auth_settings_path: Option<PathBuf>,
    pub public_bind: bool,
    pub local_webui_session_token: Option<String>,
    pub(crate) auth_settings_write: Arc<Mutex<()>>,
    pub login_attempt_limiter: LoginAttemptLimiter,
    pub metrics_include_torrent_ids: bool,
    pub categories: Arc<RwLock<BTreeMap<String, String>>>,
    pub tags: Arc<RwLock<Vec<String>>>,
    pub saved_views: Arc<RwLock<JsonMap>>,
    pub ratio_groups: Arc<RwLock<JsonMap>>,
    pub workflows: Arc<RwLock<JsonMap>>,
    pub workflow_runs: Arc<RwLock<Vec<serde_json::Value>>>,
    pub rss_rules: Arc<RwLock<JsonMap>>,
    pub user_agent: Arc<RwLock<String>>,
    pub(crate) api_metrics: Arc<ApiRuntimeMetrics>,
    pub(crate) idempotency: Arc<IdempotencyStore>,
    /// Serializes read-modify-write operations for the small TorrentNG control
    /// plane. The database command itself is serialized by the engine actor,
    /// but without this lock two HTTP writers could still lose each other's
    /// JSON map update between the read and the write.
    pub(crate) json_store_write: Arc<Mutex<()>>,
    torrent_snapshot_cache: Arc<RwLock<VecDeque<CachedTorrentSnapshot>>>,
    torrent_snapshot_refresh: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::from_parts(
            Arc::new(RwLock::new(SessionRegistry::new())),
            None,
            Vec::new(),
            ApiRuntimeMetrics::new(),
        )
    }

    fn from_parts(
        registry: Arc<RwLock<SessionRegistry>>,
        engine: Option<EngineHandle>,
        api_tokens: Vec<String>,
        api_metrics: Arc<ApiRuntimeMetrics>,
    ) -> Self {
        let default_credentials = AuthCredentials {
            username: "torrentng".to_owned(),
            password: uuid::Uuid::new_v4().simple().to_string(),
        };
        Self(Arc::new(AppStateInner {
            registry,
            engine,
            api_tokens: Arc::new(api_tokens),
            auth_credentials: Arc::new(RwLock::new(default_credentials.clone())),
            configured_auth_credentials: default_credentials,
            auth_settings_path: None,
            public_bind: false,
            local_webui_session_token: None,
            auth_settings_write: Arc::new(Mutex::new(())),
            login_attempt_limiter: LoginAttemptLimiter::default(),
            metrics_include_torrent_ids: false,
            categories: Arc::new(RwLock::new(BTreeMap::new())),
            tags: Arc::new(RwLock::new(Vec::new())),
            saved_views: Arc::new(RwLock::new(BTreeMap::new())),
            ratio_groups: Arc::new(RwLock::new(BTreeMap::new())),
            workflows: Arc::new(RwLock::new(BTreeMap::new())),
            workflow_runs: Arc::new(RwLock::new(Vec::new())),
            rss_rules: Arc::new(RwLock::new(BTreeMap::new())),
            user_agent: Arc::new(RwLock::new(rt_engine::peer_id::user_agent())),
            api_metrics,
            idempotency: IdempotencyStore::new(),
            json_store_write: Arc::new(Mutex::new(())),
            torrent_snapshot_cache: Arc::new(RwLock::new(VecDeque::new())),
            torrent_snapshot_refresh: Arc::new(Mutex::new(())),
        }))
    }

    pub fn with_registry(registry: Arc<RwLock<SessionRegistry>>) -> Self {
        Self::from_parts(registry, None, Vec::new(), ApiRuntimeMetrics::new())
    }

    pub fn with_tokens(engine: Option<EngineHandle>, api_tokens: Vec<String>) -> Self {
        Self::from_parts(
            Arc::new(RwLock::new(SessionRegistry::new())),
            engine,
            api_tokens,
            ApiRuntimeMetrics::new(),
        )
    }

    pub fn with_engine(registry: Arc<RwLock<SessionRegistry>>, engine: EngineHandle) -> Self {
        Self::from_parts(registry, Some(engine), Vec::new(), ApiRuntimeMetrics::new())
    }

    pub fn with_engine_and_tokens(
        registry: Arc<RwLock<SessionRegistry>>,
        engine: EngineHandle,
        api_tokens: Vec<String>,
    ) -> Self {
        Self::from_parts(registry, Some(engine), api_tokens, ApiRuntimeMetrics::new())
    }

    pub fn with_engine_and_tokens_and_metrics(
        registry: Arc<RwLock<SessionRegistry>>,
        engine: EngineHandle,
        api_tokens: Vec<String>,
        api_metrics: Arc<ApiRuntimeMetrics>,
    ) -> Self {
        Self::from_parts(registry, Some(engine), api_tokens, api_metrics)
    }

    pub fn with_engine_and_tokens_metrics_config(
        registry: Arc<RwLock<SessionRegistry>>,
        engine: EngineHandle,
        api_tokens: Vec<String>,
        api_metrics: Arc<ApiRuntimeMetrics>,
        include_torrent_ids: bool,
    ) -> Self {
        let mut state = Self::from_parts(registry, Some(engine), api_tokens, api_metrics);
        state.metrics_include_torrent_ids = include_torrent_ids;
        state
    }

    pub fn configure_auth_credentials(
        &mut self,
        credentials: AuthCredentials,
        configured_credentials: AuthCredentials,
        settings_path: PathBuf,
        public_bind: bool,
        local_webui_session_token: Option<String>,
    ) {
        self.auth_credentials = Arc::new(RwLock::new(credentials));
        self.configured_auth_credentials = configured_credentials;
        self.auth_settings_path = Some(settings_path);
        self.public_bind = public_bind;
        self.local_webui_session_token = local_webui_session_token;
    }

    /// Return a bounded, immutable summary snapshot for pagination and other
    /// read-heavy consumers.  A request without a cursor may reuse a recent
    /// snapshot; a request with a cursor is pinned to that exact generation.
    pub async fn torrent_snapshot(
        &self,
        requested_revision: Option<u64>,
    ) -> Result<TorrentSnapshot, TorrentSnapshotError> {
        {
            let cache = self.torrent_snapshot_cache.read().await;
            if let Some(revision) = requested_revision {
                if let Some(cached) = cache
                    .iter()
                    .find(|cached| cached.snapshot.revision == revision)
                {
                    return Ok(cached.snapshot.clone());
                }
                self.api_metrics.record_snapshot_expired();
                return Err(TorrentSnapshotError::Expired { revision });
            }

            let current_revision = self.registry.read().await.revision();
            if let Some(cached) = cache.front().filter(|cached| {
                cached.snapshot.revision == current_revision
                    || cached.generated_at.elapsed() <= TORRENT_SNAPSHOT_MAX_AGE
            }) {
                return Ok(cached.snapshot.clone());
            }
        }

        // Only one request may refresh the shared snapshot. Recheck after
        // taking the lock so concurrent list/SSE clients do not all clone the
        // registry after observing the same expired cache entry.
        let _refresh = self.torrent_snapshot_refresh.lock().await;
        {
            let cache = self.torrent_snapshot_cache.read().await;
            let current_revision = self.registry.read().await.revision();
            if let Some(cached) = cache.front().filter(|cached| {
                cached.snapshot.revision == current_revision
                    || cached.generated_at.elapsed() <= TORRENT_SNAPSHOT_MAX_AGE
            }) {
                return Ok(cached.snapshot.clone());
            }
        }

        let previous = { self.torrent_snapshot_cache.read().await.front().cloned() };
        let (revision, (torrents, filters), incremental) = {
            let registry = self.registry.read().await;
            let revision = registry.revision();
            let incremental = previous
                .filter(|cached| cached.snapshot.revision < revision)
                .and_then(|cached| {
                    changed_hashes_since(&registry, cached.snapshot.revision)
                        .map(|changed_hashes| (cached.snapshot, changed_hashes))
                });
            if let Some((previous, changed_hashes)) = incremental {
                (
                    revision,
                    {
                        let (torrents, filters) =
                            apply_snapshot_hashes(&registry, &previous, &changed_hashes);
                        (Arc::new(torrents), Arc::new(filters))
                    },
                    true,
                )
            } else {
                let session_snapshot = registry.snapshot();
                let torrents = session_snapshot
                    .iter()
                    .map(|entry| TorrentSnapshotItem {
                        summary: torrent_summary(entry),
                        amount_left: entry.amount_left,
                        name_sort_key: entry.name.to_ascii_lowercase(),
                    })
                    .collect::<Vec<_>>();
                let torrents = ChunkedVec::from_vec(torrents);
                let filters = build_filter_index(&torrents);
                (revision, (Arc::new(torrents), Arc::new(filters)), false)
            }
        };
        self.api_metrics.record_snapshot_refresh();
        if incremental {
            self.api_metrics.record_snapshot_incremental_update();
        }
        let snapshot = TorrentSnapshot {
            revision,
            torrents,
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters,
        };
        let mut cache = self.torrent_snapshot_cache.write().await;
        cache.retain(|cached| cached.snapshot.revision != revision);
        cache.push_front(CachedTorrentSnapshot {
            snapshot: snapshot.clone(),
            generated_at: Instant::now(),
        });
        cache.truncate(TORRENT_SNAPSHOT_CACHE_SIZE);
        Ok(snapshot)
    }
}

#[cfg(test)]
fn apply_snapshot_changes(
    registry: &SessionRegistry,
    previous: &TorrentSnapshot,
    changes: &[RegistryChange],
) -> (ChunkedVec<TorrentSnapshotItem>, TorrentFilterIndex) {
    let changed_hashes = changes
        .iter()
        .map(|change| CompactHashKey::from_hash(&change.info_hash))
        .collect::<HashSet<_>>();
    apply_snapshot_hashes(registry, previous, &changed_hashes)
}

fn changed_hashes_since(
    registry: &SessionRegistry,
    revision: u64,
) -> Option<HashSet<CompactHashKey>> {
    let mut cursor = revision;
    let mut changed_hashes = HashSet::new();
    loop {
        let page = registry.delta_since_page(cursor, REGISTRY_DELTA_PAGE_MAX)?;
        changed_hashes.extend(
            page.changed
                .into_iter()
                .chain(page.removed)
                .map(|hash| CompactHashKey::from_hash(&hash)),
        );
        cursor = page.through_revision;
        if !page.has_more {
            return (cursor == page.current_revision).then_some(changed_hashes);
        }
    }
}

fn apply_snapshot_hashes(
    registry: &SessionRegistry,
    previous: &TorrentSnapshot,
    changed_hashes: &HashSet<CompactHashKey>,
) -> (ChunkedVec<TorrentSnapshotItem>, TorrentFilterIndex) {
    let mut replacements = Vec::new();
    let mut structural_change = false;
    for hash_key in changed_hashes {
        let previous_index = previous.filters.by_hash.get(hash_key).copied();
        let lookup = previous_index
            .and_then(|index| previous.torrents.get(index))
            .map(|item| Cow::Borrowed(item.summary.info_hash.as_str()))
            .unwrap_or_else(|| hash_key.lookup_text());
        match registry.get(lookup.as_ref()) {
            Some(entry) => {
                let item = TorrentSnapshotItem {
                    summary: torrent_summary(&entry),
                    amount_left: entry.amount_left,
                    name_sort_key: entry.name.to_ascii_lowercase(),
                };
                if let Some(index) = previous_index {
                    if let Some(old) = previous.torrents.get(index) {
                        replacements.push((index, old.clone(), item));
                    } else {
                        structural_change = true;
                    }
                } else {
                    structural_change = true;
                }
            }
            None => {
                if previous_index.is_some() {
                    structural_change = true;
                }
            }
        }
    }

    if structural_change {
        // A membership change invalidates positional indexes. Mutating the
        // previous vector in HashSet iteration order can apply a later
        // replacement at a stale index after an earlier removal, producing
        // corrupted rows or an out-of-bounds panic. Rebuild from the
        // registry's deterministic snapshot instead.
        let session_snapshot = registry.snapshot();
        let torrents = session_snapshot
            .iter()
            .map(|entry| TorrentSnapshotItem {
                summary: torrent_summary(entry),
                amount_left: entry.amount_left,
                name_sort_key: entry.name.to_ascii_lowercase(),
            })
            .collect::<Vec<_>>();
        let torrents = ChunkedVec::from_vec(torrents);
        let filters = build_filter_index(&torrents);
        (torrents, filters)
    } else {
        let mut filters = previous.filters.as_ref().clone();
        let mut values = Vec::with_capacity(replacements.len());
        for (index, old, item) in replacements {
            update_filter_index(&mut filters, index, &old, &item, previous.torrents.len());
            values.push((index, item));
        }
        (previous.torrents.replace_many(values), filters)
    }
}

impl TorrentSnapshot {
    /// Return candidate indexes for exact operational filters. The returned
    /// indexes are sorted in snapshot order, so callers can intersect them
    /// and use binary search while traversing a requested sort order.
    pub(crate) fn candidate_indices(
        &self,
        states: &[&str],
        category: Option<&str>,
        tag: Option<&str>,
        filter: Option<&str>,
        media_type: Option<&str>,
    ) -> Option<Vec<usize>> {
        let mut candidates = if states.is_empty() {
            None
        } else {
            let mut union = Vec::new();
            for state in states {
                if let Some(indexes) = self.filters.by_state.get(*state) {
                    union.extend(indexes.indices());
                }
            }
            union.sort_unstable();
            union.dedup();
            Some(union)
        };

        if let Some(category) = category {
            let Some(indexes) = self.filters.by_category.get(category) else {
                return Some(Vec::new());
            };
            candidates = Some(match candidates {
                Some(current) => intersect_sorted(&current, &indexes.indices()),
                None => indexes.indices(),
            });
        }
        if let Some(tag) = tag {
            let Some(indexes) = self.filters.by_tag.get(tag) else {
                return Some(Vec::new());
            };
            candidates = Some(match candidates {
                Some(current) => intersect_sorted(&current, &indexes.indices()),
                None => indexes.indices(),
            });
        }
        if let Some(media_type) = media_type.map(str::trim).filter(|value| !value.is_empty()) {
            let media_type = media_type.to_ascii_lowercase();
            let Some(indexes) = self.filters.by_media_type.get(media_type.as_str()) else {
                return Some(Vec::new());
            };
            candidates = Some(match candidates {
                Some(current) => intersect_sorted(&current, &indexes.indices()),
                None => indexes.indices(),
            });
        }
        if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
            let (bytes, bigrams, trigrams) = search_text_ngrams(filter);
            for byte in bytes {
                let Some(indexes) = self.filters.by_text_byte.get(&byte) else {
                    return Some(Vec::new());
                };
                candidates = Some(match candidates {
                    Some(current) => intersect_sorted(&current, &indexes.indices()),
                    None => indexes.indices(),
                });
            }
            for bigram in bigrams {
                let Some(indexes) = self.filters.by_text_bigram.get(&bigram) else {
                    return Some(Vec::new());
                };
                candidates = Some(match candidates {
                    Some(current) => intersect_sorted(&current, &indexes.indices()),
                    None => indexes.indices(),
                });
            }
            for trigram in trigrams {
                let Some(indexes) = self.filters.by_text_trigram.get(&trigram) else {
                    return Some(Vec::new());
                };
                candidates = Some(match candidates {
                    Some(current) => intersect_sorted(&current, &indexes.indices()),
                    None => indexes.indices(),
                });
            }
        }
        candidates
    }

    pub(crate) fn category_facets(&self) -> impl Iterator<Item = TorrentLabelFacet> + '_ {
        self.filters
            .by_category
            .iter()
            .map(|(name, indexes)| TorrentLabelFacet {
                name: name.clone(),
                count: indexes.count(),
                save_path: indexes
                    .first_index()
                    .and_then(|index| self.torrents.get(index))
                    .map(|item| item.summary.save_path.clone()),
            })
    }

    pub(crate) fn tag_facets(&self) -> impl Iterator<Item = (String, usize)> + '_ {
        self.filters
            .by_tag
            .iter()
            .map(|(name, indexes)| (name.clone(), indexes.count()))
    }
}

fn build_filter_index(torrents: &ChunkedVec<TorrentSnapshotItem>) -> TorrentFilterIndex {
    let mut by_hash = HashMap::<CompactHashKey, usize>::new();
    let mut by_state = HashMap::<String, Vec<usize>>::new();
    let mut by_media_type = HashMap::<String, Vec<usize>>::new();
    let mut by_category = HashMap::<String, Vec<usize>>::new();
    let mut by_tag = HashMap::<String, Vec<usize>>::new();
    let mut by_text_byte = HashMap::<u8, Vec<usize>>::new();
    let mut by_text_bigram = HashMap::<[u8; 2], Vec<usize>>::new();
    let mut by_text_trigram = HashMap::<[u8; 3], Vec<usize>>::new();
    for (index, item) in torrents.iter().enumerate() {
        by_hash.insert(CompactHashKey::from_hash(&item.summary.info_hash), index);
        add_text_ngrams(
            &item.summary.name,
            index,
            &mut by_text_byte,
            &mut by_text_bigram,
            &mut by_text_trigram,
        );
        add_text_ngrams(
            &item.summary.info_hash,
            index,
            &mut by_text_byte,
            &mut by_text_bigram,
            &mut by_text_trigram,
        );
        by_state
            .entry(item.summary.state.clone())
            .or_default()
            .push(index);
        by_media_type
            .entry(infer_media_type(&item.summary).to_owned())
            .or_default()
            .push(index);
        if let Some(category) = &item.summary.category {
            by_category.entry(category.clone()).or_default().push(index);
        }
        let mut seen_tags = HashSet::new();
        for tag in &item.summary.tags {
            if seen_tags.insert(tag) {
                by_tag.entry(tag.clone()).or_default().push(index);
            }
        }
    }
    TorrentFilterIndex {
        by_hash: Arc::new(by_hash),
        by_state: by_state
            .into_iter()
            .map(|(key, indexes)| {
                (
                    key,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_media_type: by_media_type
            .into_iter()
            .map(|(key, indexes)| {
                (
                    key,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_category: by_category
            .into_iter()
            .map(|(key, indexes)| {
                (
                    key,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_tag: by_tag
            .into_iter()
            .map(|(key, indexes)| {
                (
                    key,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_text_byte: by_text_byte
            .into_iter()
            .map(|(byte, indexes)| {
                (
                    byte,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_text_bigram: by_text_bigram
            .into_iter()
            .map(|(bigram, indexes)| {
                (
                    bigram,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
        by_text_trigram: by_text_trigram
            .into_iter()
            .map(|(trigram, indexes)| {
                (
                    trigram,
                    Arc::new(ChunkedBitSet::from_indices(torrents.len(), indexes)),
                )
            })
            .collect(),
    }
}

fn update_filter_index(
    filters: &mut TorrentFilterIndex,
    index: usize,
    old: &TorrentSnapshotItem,
    new: &TorrentSnapshotItem,
    len: usize,
) {
    update_membership(
        &mut filters.by_state,
        Some(old.summary.state.as_str()),
        Some(new.summary.state.as_str()),
        index,
        len,
    );
    update_membership(
        &mut filters.by_media_type,
        Some(infer_media_type(&old.summary)),
        Some(infer_media_type(&new.summary)),
        index,
        len,
    );
    update_membership(
        &mut filters.by_category,
        old.summary.category.as_deref(),
        new.summary.category.as_deref(),
        index,
        len,
    );

    let old_tags = old
        .summary
        .tags
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let new_tags = new
        .summary
        .tags
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for tag in old_tags.union(&new_tags) {
        update_membership(
            &mut filters.by_tag,
            old_tags.contains(tag).then_some(*tag),
            new_tags.contains(tag).then_some(*tag),
            index,
            len,
        );
    }

    let (old_bytes, old_bigrams, old_trigrams) = search_text_ngrams(&old.summary.name);
    let (new_bytes, new_bigrams, new_trigrams) = search_text_ngrams(&new.summary.name);
    for byte in old_bytes.union(&new_bytes) {
        update_text_membership(
            &mut filters.by_text_byte,
            old_bytes.contains(byte).then_some(byte),
            new_bytes.contains(byte).then_some(byte),
            index,
            len,
        );
    }
    for bigram in old_bigrams.union(&new_bigrams) {
        update_text_membership(
            &mut filters.by_text_bigram,
            old_bigrams.contains(bigram).then_some(bigram),
            new_bigrams.contains(bigram).then_some(bigram),
            index,
            len,
        );
    }
    for trigram in old_trigrams.union(&new_trigrams) {
        update_text_membership(
            &mut filters.by_text_trigram,
            old_trigrams.contains(trigram).then_some(trigram),
            new_trigrams.contains(trigram).then_some(trigram),
            index,
            len,
        );
    }
}

fn search_text_ngrams(value: &str) -> (HashSet<u8>, HashSet<[u8; 2]>, HashSet<[u8; 3]>) {
    let value = value.to_ascii_lowercase();
    let bytes = value.as_bytes();
    (
        bytes.iter().copied().collect(),
        bytes
            .windows(2)
            .map(|window| [window[0], window[1]])
            .collect(),
        bytes
            .windows(3)
            .map(|window| [window[0], window[1], window[2]])
            .collect(),
    )
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(index, _)| {
        let before_ok = haystack.as_bytes()[..index]
            .last()
            .map(|byte| !is_word_byte(*byte))
            .unwrap_or(true);
        let end = index + needle.len();
        let after_ok = haystack.as_bytes()[end..]
            .first()
            .map(|byte| !is_word_byte(*byte))
            .unwrap_or(true);
        before_ok && after_ok
    })
}

fn contains_extension(haystack: &str, extension: &str) -> bool {
    haystack.match_indices(extension).any(|(index, _)| {
        let end = index + extension.len();
        haystack.as_bytes()[end..]
            .first()
            .map(|byte| !is_word_byte(*byte))
            .unwrap_or(true)
    })
}

fn contains_any_word(haystack: &str, words: &[&str]) -> bool {
    words.iter().any(|word| contains_word(haystack, word))
}

fn contains_any_extension(haystack: &str, extensions: &[&str]) -> bool {
    extensions
        .iter()
        .any(|extension| contains_extension(haystack, extension))
}

fn is_ebook(haystack: &str) -> bool {
    contains_any_word(haystack, &["ebook", "ebooks", "book", "books", "audiobook"])
        || contains_any_extension(
            haystack,
            &[".epub", ".mobi", ".azw3", ".pdf", ".cbz", ".cbr"],
        )
}

fn contains_season_episode(haystack: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b's' && (index == 0 || !is_word_byte(bytes[index - 1])) {
            let season_start = index + 1;
            let mut cursor = season_start;
            while cursor < bytes.len()
                && bytes[cursor].is_ascii_digit()
                && cursor - season_start < 2
            {
                cursor += 1;
            }
            if cursor > season_start && cursor < bytes.len() && bytes[cursor] == b'e' {
                let episode_start = cursor + 1;
                let mut end = episode_start;
                while end < bytes.len() && bytes[end].is_ascii_digit() && end - episode_start < 3 {
                    end += 1;
                }
                if end > episode_start && (end == bytes.len() || !is_word_byte(bytes[end])) {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

pub(crate) fn infer_media_type(summary: &TorrentSummary) -> &'static str {
    let category = summary.category.as_deref().unwrap_or_default();
    let tags = summary.tags.join(" ");
    let haystack = format!(
        "{} {} {} {}",
        summary.name, category, tags, summary.save_path
    )
    .to_ascii_lowercase();

    if is_ebook(&haystack) {
        "ebook"
    } else if contains_any_word(
        &haystack,
        &["season", "episode", "hdtv", "web-dl", "webrip", "tv"],
    ) || contains_season_episode(&haystack)
    {
        "tv"
    } else if contains_any_word(
        &haystack,
        &[
            "movie", "movies", "film", "bluray", "bdrip", "dvdrip", "x264", "x265", "h.264",
            "h.265", "2160p", "1080p", "720p",
        ],
    ) || contains_any_extension(
        &haystack,
        &[".mkv", ".mp4", ".avi", ".mov", ".wmv", ".m4v"],
    ) {
        "video"
    } else if contains_any_word(&haystack, &["music", "album", "discography"])
        || contains_any_extension(
            &haystack,
            &[".flac", ".mp3", ".aac", ".ogg", ".opus", ".wav", ".m4a"],
        )
    {
        "audio"
    } else if contains_any_word(
        &haystack,
        &[
            "iso",
            "installer",
            "image",
            "linux",
            "ubuntu",
            "debian",
            "archlinux",
            "fedora",
        ],
    ) || contains_any_extension(&haystack, &[".iso", ".img", ".dmg"])
    {
        "image"
    } else if contains_any_word(
        &haystack,
        &[
            "game", "games", "gog", "steam", "switch", "ps4", "ps5", "xbox",
        ],
    ) {
        "game"
    } else if contains_any_word(
        &haystack,
        &[
            "app", "software", "source", "code", "github", "windows", "macos", "linux",
        ],
    ) || contains_any_extension(
        &haystack,
        &[
            ".exe", ".msi", ".pkg", ".deb", ".rpm", ".zip", ".tar", ".gz", ".xz", ".7z", ".rar",
        ],
    ) {
        "software"
    } else {
        "other"
    }
}

fn add_text_ngrams(
    value: &str,
    index: usize,
    bytes: &mut HashMap<u8, Vec<usize>>,
    bigrams: &mut HashMap<[u8; 2], Vec<usize>>,
    trigrams: &mut HashMap<[u8; 3], Vec<usize>>,
) {
    let (value_bytes, value_bigrams, value_trigrams) = search_text_ngrams(value);
    for byte in value_bytes {
        bytes.entry(byte).or_default().push(index);
    }
    for bigram in value_bigrams {
        bigrams.entry(bigram).or_default().push(index);
    }
    for trigram in value_trigrams {
        trigrams.entry(trigram).or_default().push(index);
    }
}

fn update_membership(
    index: &mut HashMap<String, Arc<ChunkedBitSet>>,
    old: Option<&str>,
    new: Option<&str>,
    position: usize,
    len: usize,
) {
    if old == new {
        return;
    }
    if let Some(old) = old {
        if let Some(bits) = index.get_mut(old) {
            *bits = Arc::new(bits.set(position, false));
        }
    }
    if let Some(new) = new {
        let bits = index
            .entry(new.to_owned())
            .or_insert_with(|| Arc::new(ChunkedBitSet::empty(len)));
        *bits = Arc::new(bits.set(position, true));
    }
}

fn update_text_membership<K>(
    index: &mut HashMap<K, Arc<ChunkedBitSet>>,
    old: Option<&K>,
    new: Option<&K>,
    position: usize,
    len: usize,
) where
    K: Clone + Eq + std::hash::Hash,
{
    if old == new {
        return;
    }
    if let Some(old) = old {
        if let Some(bits) = index.get_mut(old) {
            *bits = Arc::new(bits.set(position, false));
        }
    }
    if let Some(new) = new {
        let bits = index
            .entry(new.clone())
            .or_insert_with(|| Arc::new(ChunkedBitSet::empty(len)));
        *bits = Arc::new(bits.set(position, true));
    }
}

fn intersect_sorted(left: &[usize], right: &[usize]) -> Vec<usize> {
    let mut result = Vec::with_capacity(left.len().min(right.len()));
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                result.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    result
}

impl TorrentSnapshot {
    /// Return an immutable ascending index for a supported sort. The index is
    /// built once per cached snapshot and reused by every page request that
    /// references that snapshot; reverse order is handled by the caller.
    pub(crate) fn ordered_indices(&self, requested_sort: Option<&str>) -> Arc<Vec<usize>> {
        let sort = match requested_sort.unwrap_or("added") {
            "name" => "name",
            "size" | "total_length" => "size",
            "ratio" => "ratio",
            "progress" => "progress",
            "hash" => "hash",
            _ => "added",
        };
        if let Some(indices) = lock_order_cache(&self.orders).get(sort).cloned() {
            return indices;
        }

        let mut indices = (0..self.torrents.len()).collect::<Vec<_>>();
        indices.sort_unstable_by(|left, right| {
            let left = self.torrents.get(*left).expect("snapshot index is valid");
            let right = self.torrents.get(*right).expect("snapshot index is valid");
            let ordering = match sort {
                "name" => left.name_sort_key.cmp(&right.name_sort_key),
                "size" => left.summary.total_length.cmp(&right.summary.total_length),
                "ratio" => left
                    .summary
                    .ratio
                    .partial_cmp(&right.summary.ratio)
                    .unwrap_or(Ordering::Equal),
                "progress" => progress(left).cmp(&progress(right)),
                "hash" => left.summary.info_hash.cmp(&right.summary.info_hash),
                _ => left.summary.added_at.cmp(&right.summary.added_at),
            };
            ordering.then_with(|| left.summary.info_hash.cmp(&right.summary.info_hash))
        });
        let indices = Arc::new(indices);
        lock_order_cache(&self.orders).insert(sort, Arc::clone(&indices));
        indices
    }
}

fn progress(item: &TorrentSnapshotItem) -> u64 {
    item.summary
        .total_length
        .max(0)
        .try_into()
        .unwrap_or(u64::MAX)
        .saturating_sub(item.amount_left)
}

pub(crate) fn torrent_summary(entry: &rt_session::TorrentEntry) -> TorrentSummary {
    TorrentSummary {
        info_hash: entry.info_hash.clone(),
        name: entry.name.clone(),
        state: entry.state.as_str().to_owned(),
        total_length: torrentng_i64(entry.total_length),
        downloaded: torrentng_i64(entry.stats.downloaded),
        amount_left: torrentng_i64(entry.amount_left),
        uploaded: torrentng_i64(entry.stats.uploaded),
        ratio: entry.stats.ratio(),
        save_path: entry.save_path.clone(),
        category: entry.category.clone(),
        tags: entry.tags.clone(),
        added_at: torrentng_i64(entry.added_at),
        completed_at: entry.completed_at.map(torrentng_i64),
        num_peers: 0,
        num_seeds: 0,
        tracker_message: entry.tracker_message.clone(),
        finalizing: entry.finalizing,
    }
}

/// API models use signed counters for compatibility with the existing wire
/// contract. Never let a persisted or engine-owned u64 wrap into a negative
/// value when projecting it to that contract.
pub(crate) fn torrentng_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn torrentng_usize_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rt_session::TorrentEntry;

    #[tokio::test]
    async fn state_clones_are_compact_and_configuration_is_copy_on_write() {
        let original = AppState::new();
        let mut changed = original.clone();
        assert_eq!(
            std::mem::size_of::<AppState>(),
            std::mem::size_of::<Arc<AppStateInner>>()
        );
        assert!(Arc::ptr_eq(&original.0, &changed.0));
        changed.metrics_include_torrent_ids = !original.metrics_include_torrent_ids;
        changed.configure_auth_credentials(
            AuthCredentials {
                username: "changed".into(),
                password: "changed-password".into(),
            },
            AuthCredentials {
                username: "configured".into(),
                password: "configured-password".into(),
            },
            PathBuf::from("changed-auth.json"),
            true,
            Some("changed-token".into()),
        );
        assert!(!Arc::ptr_eq(&original.0, &changed.0));
        assert_ne!(
            original.metrics_include_torrent_ids,
            changed.metrics_include_torrent_ids
        );
        assert_eq!(original.configured_auth_credentials.username, "torrentng");
        assert_eq!(original.auth_settings_path, None);
        assert_eq!(original.local_webui_session_token, None);
        assert!(!original.public_bind);
        assert_eq!(original.auth_credentials.read().await.username, "torrentng");
        assert_eq!(changed.auth_credentials.read().await.username, "changed");
        assert!(Arc::ptr_eq(&original.registry, &changed.registry));
        assert!(Arc::ptr_eq(&original.categories, &changed.categories));
        assert!(Arc::ptr_eq(&original.idempotency, &changed.idempotency));
        assert!(Arc::ptr_eq(
            &original.torrent_snapshot_cache,
            &changed.torrent_snapshot_cache
        ));
        assert!(Arc::ptr_eq(
            &original.login_attempt_limiter.state,
            &changed.login_attempt_limiter.state
        ));
        changed
            .categories
            .write()
            .await
            .insert("shared".into(), "/data".into());
        assert_eq!(
            original
                .categories
                .read()
                .await
                .get("shared")
                .map(String::as_str),
            Some("/data")
        );
    }

    #[test]
    fn snapshot_hash_keys_compact_canonical_hashes_and_preserve_other_text() {
        let v1 = "ab".repeat(20);
        let v1_key = CompactHashKey::from_hash(&v1);
        assert!(matches!(&v1_key, CompactHashKey::V1(_)));
        assert_eq!(v1_key.lookup_text().as_ref(), v1.as_str());

        let v2 = "cd".repeat(32);
        let v2_key = CompactHashKey::from_hash(&v2);
        assert!(matches!(&v2_key, CompactHashKey::V2(_)));
        assert_eq!(v2_key.lookup_text().as_ref(), v2.as_str());

        let placeholder = "TestHash-Upper";
        let other_key = CompactHashKey::from_hash(placeholder);
        assert!(matches!(&other_key, CompactHashKey::Other(_)));
        assert_eq!(other_key.lookup_text().as_ref(), placeholder);

        let uppercase_hash = v1.to_ascii_uppercase();
        assert!(matches!(
            CompactHashKey::from_hash(&uppercase_hash),
            CompactHashKey::Other(_)
        ));
    }

    #[tokio::test]
    async fn login_attempt_limiter_bounds_failures_and_clears_after_success() {
        let limiter = LoginAttemptLimiter::default();
        let peer = Some("192.0.2.1".parse::<IpAddr>().unwrap());
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            assert!(limiter.begin_attempt(peer).await.is_ok());
        }
        assert!(limiter.begin_attempt(peer).await.is_err());
        limiter.clear(peer).await;
        assert!(limiter.begin_attempt(peer).await.is_ok());
    }

    #[test]
    fn journal_refresh_applies_final_entry_state_without_registry_scan() {
        let first_hash = "a".repeat(40);
        let second_hash = "b".repeat(40);
        let mut registry = SessionRegistry::new();
        registry
            .add(TorrentEntry::new(
                first_hash.clone(),
                "before".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();
        let previous_items = registry
            .iter()
            .map(|entry| TorrentSnapshotItem {
                summary: torrent_summary(&entry),
                amount_left: entry.amount_left,
                name_sort_key: entry.name.to_ascii_lowercase(),
            })
            .collect::<Vec<_>>();
        let previous_items = ChunkedVec::from_vec(previous_items.clone());
        let previous = TorrentSnapshot {
            revision: registry.revision(),
            torrents: Arc::new(previous_items.clone()),
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters: Arc::new(build_filter_index(&previous_items)),
        };

        registry
            .add(TorrentEntry::new(
                second_hash.clone(),
                "removed".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();
        {
            let mut entry = registry.get_mut(&first_hash).unwrap();
            entry.name = "after".to_owned();
        }
        registry.remove(&second_hash).unwrap();

        let changes = registry.changes_since(previous.revision).unwrap();
        let refreshed = apply_snapshot_changes(&registry, &previous, &changes);
        assert_eq!(refreshed.0.len(), 1);
        assert_eq!(refreshed.0.get(0).unwrap().summary.info_hash, first_hash);
        assert_eq!(refreshed.0.get(0).unwrap().summary.name, "after");
        let refreshed_snapshot = TorrentSnapshot {
            revision: registry.revision(),
            torrents: Arc::new(refreshed.0),
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters: Arc::new(refreshed.1),
        };
        assert_eq!(
            refreshed_snapshot.candidate_indices(&[], None, None, Some("after"), None),
            Some(vec![0])
        );
    }

    #[test]
    fn structural_snapshot_refresh_does_not_use_stale_positions() {
        let first_hash = "a".repeat(40);
        let second_hash = "b".repeat(40);
        let mut registry = SessionRegistry::new();
        registry
            .add(TorrentEntry::new(
                first_hash.clone(),
                "before".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();
        registry
            .add(TorrentEntry::new(
                second_hash.clone(),
                "removed".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();
        let session_snapshot = registry.snapshot();
        let previous_items = session_snapshot
            .iter()
            .map(|entry| TorrentSnapshotItem {
                summary: torrent_summary(entry),
                amount_left: entry.amount_left,
                name_sort_key: entry.name.to_ascii_lowercase(),
            })
            .collect::<Vec<_>>();
        let previous_items = ChunkedVec::from_vec(previous_items);
        let previous = TorrentSnapshot {
            revision: registry.revision(),
            torrents: Arc::new(previous_items.clone()),
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters: Arc::new(build_filter_index(&previous_items)),
        };

        registry.get_mut(&first_hash).unwrap().name = "after".to_owned();
        registry.remove(&second_hash).unwrap();

        // Deliberately place the removal before the replacement. The old
        // positional implementation could index past the shortened vector.
        let changes = vec![
            RegistryChange {
                revision: previous.revision + 1,
                info_hash: second_hash,
                removed: true,
            },
            RegistryChange {
                revision: previous.revision + 2,
                info_hash: first_hash.clone(),
                removed: false,
            },
        ];
        let (torrents, _) = apply_snapshot_changes(&registry, &previous, &changes);
        assert_eq!(torrents.len(), 1);
        assert_eq!(torrents.get(0).unwrap().summary.info_hash, first_hash);
        assert_eq!(torrents.get(0).unwrap().summary.name, "after");
    }

    #[test]
    fn signed_summary_projection_saturates_unsigned_counters() {
        assert_eq!(torrentng_i64(i64::MAX as u64), i64::MAX);
        assert_eq!(torrentng_i64(i64::MAX as u64 + 1), i64::MAX);
        assert_eq!(torrentng_usize_i64(usize::MAX), i64::MAX);
    }

    #[test]
    fn text_filter_index_handles_short_filters_and_case_folding() {
        let first_hash = "a".repeat(40);
        let second_hash = "b".repeat(40);
        let mut registry = SessionRegistry::new();
        registry
            .add(TorrentEntry::new(
                first_hash,
                "Alpha Release".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();
        registry
            .add(TorrentEntry::new(
                second_hash,
                "Zulu Release".to_owned(),
                "/data".to_owned(),
            ))
            .unwrap();

        let items = registry
            .iter()
            .map(|entry| TorrentSnapshotItem {
                summary: torrent_summary(&entry),
                amount_left: entry.amount_left,
                name_sort_key: entry.name.to_ascii_lowercase(),
            })
            .collect::<Vec<_>>();
        let items = ChunkedVec::from_vec(items);
        let snapshot = TorrentSnapshot {
            revision: registry.revision(),
            torrents: Arc::new(items.clone()),
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters: Arc::new(build_filter_index(&items)),
        };
        let alpha_index = snapshot
            .torrents
            .iter()
            .position(|item| item.summary.name == "Alpha Release")
            .unwrap();
        let zulu_index = snapshot
            .torrents
            .iter()
            .position(|item| item.summary.name == "Zulu Release")
            .unwrap();

        assert_eq!(
            snapshot.candidate_indices(&[], None, None, Some("P"), None),
            Some(vec![alpha_index])
        );
        assert_eq!(
            snapshot.candidate_indices(&[], None, None, Some("ph"), None),
            Some(vec![alpha_index])
        );
        let release_candidates = snapshot
            .candidate_indices(&[], None, None, Some("release"), None)
            .unwrap();
        assert_eq!(
            release_candidates,
            vec![alpha_index.min(zulu_index), alpha_index.max(zulu_index)]
        );
        assert_eq!(
            snapshot.candidate_indices(&[], None, None, Some("qq"), None),
            Some(Vec::new())
        );
        assert_eq!(
            snapshot.candidate_indices(&[], None, None, Some("   "), None),
            None
        );
    }

    #[test]
    fn poisoned_sort_cache_is_discarded_and_rebuilt_from_snapshot() {
        let summary = |info_hash: &str, name: &str| TorrentSummary {
            info_hash: info_hash.to_owned(),
            name: name.to_owned(),
            state: "stopped".to_owned(),
            total_length: 0,
            downloaded: 0,
            amount_left: 0,
            uploaded: 0,
            ratio: 0.0,
            save_path: "/data".to_owned(),
            category: None,
            tags: Vec::new(),
            added_at: 0,
            completed_at: None,
            num_peers: 0,
            num_seeds: 0,
            tracker_message: None,
            finalizing: false,
        };
        let items = ChunkedVec::from_vec(vec![
            TorrentSnapshotItem {
                summary: summary("b", "Zulu"),
                amount_left: 0,
                name_sort_key: "zulu".to_owned(),
            },
            TorrentSnapshotItem {
                summary: summary("a", "Alpha"),
                amount_left: 0,
                name_sort_key: "alpha".to_owned(),
            },
        ]);
        let snapshot = TorrentSnapshot {
            revision: 1,
            torrents: Arc::new(items.clone()),
            orders: Arc::new(StdMutex::new(HashMap::new())),
            filters: Arc::new(build_filter_index(&items)),
        };

        let cache = Arc::clone(&snapshot.orders);
        assert!(std::thread::spawn(move || {
            let _guard = cache.lock().unwrap();
            panic!("poison the derived sort cache");
        })
        .join()
        .is_err());
        assert!(snapshot.orders.is_poisoned());

        let ordered = snapshot.ordered_indices(Some("name"));
        assert_eq!(ordered.as_slice(), &[1, 0]);
        assert!(Arc::ptr_eq(
            &ordered,
            &snapshot.ordered_indices(Some("name"))
        ));
        assert!(!snapshot.orders.is_poisoned());
    }

    #[test]
    fn media_type_facets_are_indexed_and_updated_incrementally() {
        let summary = |name: &str| TorrentSummary {
            info_hash: name.to_owned(),
            name: name.to_owned(),
            state: "stopped".to_owned(),
            total_length: 0,
            downloaded: 0,
            amount_left: 0,
            uploaded: 0,
            ratio: 0.0,
            save_path: "/data".to_owned(),
            category: None,
            tags: Vec::new(),
            added_at: 0,
            completed_at: None,
            num_peers: 0,
            num_seeds: 0,
            tracker_message: None,
            finalizing: false,
        };
        let items = ChunkedVec::from_vec(vec![
            TorrentSnapshotItem {
                summary: summary("movie.mkv"),
                amount_left: 0,
                name_sort_key: "movie.mkv".to_owned(),
            },
            TorrentSnapshotItem {
                summary: summary("song.mp3"),
                amount_left: 0,
                name_sort_key: "song.mp3".to_owned(),
            },
        ]);
        let mut filters = build_filter_index(&items);
        let counts = |filters: &TorrentFilterIndex| {
            filters
                .by_media_type
                .iter()
                .map(|(name, indexes)| (name.clone(), indexes.count()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(counts(&filters).get("video"), Some(&1));
        assert_eq!(counts(&filters).get("audio"), Some(&1));

        let old = items.get(1).unwrap().clone();
        let new = TorrentSnapshotItem {
            summary: TorrentSummary {
                name: "archive.zip".to_owned(),
                ..old.summary.clone()
            },
            amount_left: old.amount_left,
            name_sort_key: "archive.zip".to_owned(),
        };
        update_filter_index(&mut filters, 1, &old, &new, items.len());
        assert_eq!(counts(&filters).get("audio"), Some(&0));
        assert_eq!(counts(&filters).get("software"), Some(&1));
    }
}
