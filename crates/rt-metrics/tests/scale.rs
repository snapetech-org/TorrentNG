use std::collections::BTreeMap;
/// Scale-shaped regressions and live native-engine catalog observations.
///
/// Tests prefixed `synthetic_` exercise API and database object shapes only;
/// they are not production-capacity evidence. The live catalog cases below
/// add valid metainfo through the production engine and then query the native
/// API over its populated registry.
///
/// Legacy synthetic proxy envelopes (from CLAUDE.md benchmarks section;
/// they are not production capacity targets):
///   - 1k torrents:  GET /api/v1/torrents < 100ms
///   - 10k torrents: GET /api/v1/torrents < 200ms
///   - 15k torrents: GET /api/v1/torrents < 500ms
///   - 50k torrents: GET /api/qb/v2/torrents/info < 500ms
///   - TorrentNG-client filter/sort over 15k torrents < 250ms
///   - sync/maindata delta < 50ms (at normal churn)
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{body::Body, http::Request};
use rt_api_native::{build_router, AppState};
use rt_api_qbit::AppState as QbState;
use rt_config::Config;
use rt_engine::Engine;
use rt_fastresume::{FastresumeState, ImportPolicy, PieceState};
use rt_path::{SafeRelPath, StorageProfile, StorageRootId};
use rt_piece_map::{FileSpan, PieceMap};
use rt_session::{SessionRegistry, TorrentEntry, TorrentState};
use rt_storage::{
    IoClass, MountScheduler, PreallocationMode, SchedulerConfig, StorageError, StorageIoConfig,
    VerifyResult,
};
use rt_tracker::backoff::jitter_interval;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tower::ServiceExt;

/// Build a TorrentNG API app populated with `n` synthetic torrents.
async fn torrentng_app_with(n: usize) -> axum::Router {
    let state = AppState::new();
    {
        let mut reg = state.registry.write().await;
        for i in 0..n {
            let hash = format!("{i:040x}");
            let entry = TorrentEntry::new(hash, format!("torrent_{i}"), "/data".into());
            reg.add(entry).unwrap();
        }
    }
    build_router(state)
}

/// Build a qBit API app populated with `n` synthetic torrents.
async fn qbit_app_with(n: usize) -> axum::Router {
    let state = QbState::new();
    {
        let mut reg = state.registry.write().await;
        for i in 0..n {
            let hash = format!("{i:040x}");
            let entry = TorrentEntry::new(hash, format!("torrent_{i}"), "/data".into());
            reg.add(entry).unwrap();
        }
    }
    rt_api_qbit::build_qbit_router(state)
}

/// Debug builds can be substantially slower on shared CI runners; these are
/// release-envelope smoke tests, not a benchmark of an unoptimized binary.
/// Keep the release thresholds unchanged and give debug/MSRV jobs enough
/// scheduler headroom to avoid turning host contention into a false failure.
fn threshold(release_ms: u128) -> u128 {
    if cfg!(debug_assertions) {
        release_ms * 50
    } else {
        release_ms
    }
}

async fn read_at_retry_queue_full(
    scheduler: &MountScheduler,
    class: IoClass,
    path: &Path,
    offset: u64,
    len: usize,
) -> Result<bytes::Bytes, StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.read_at(class, path, offset, len).await {
            Err(StorageError::QueueFull { .. }) if attempts < 10_000 => {
                attempts += 1;
                if attempts % 16 == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                } else {
                    tokio::task::yield_now().await;
                }
            }
            result => return result,
        }
    }
}

async fn hash_sha1_retry_queue_full(
    scheduler: &MountScheduler,
    data: bytes::Bytes,
) -> Result<[u8; 20], StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.hash_sha1(data.clone()).await {
            Err(StorageError::QueueFull { .. }) if attempts < 10_000 => {
                attempts += 1;
                if attempts % 16 == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                } else {
                    tokio::task::yield_now().await;
                }
            }
            result => return result,
        }
    }
}

async fn get_ms(app: axum::Router, uri: &str) -> u128 {
    let t0 = Instant::now();
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    // Drain body so timing includes serialization.
    let _ = axum::body::to_bytes(resp.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap();
    let elapsed_ms = t0.elapsed().as_millis();
    println!("scale uri={uri} elapsed_ms={elapsed_ms}");
    elapsed_ms
}

async fn native_list_ms_and_total(app: axum::Router, uri: &str) -> Result<(u128, usize), String> {
    let started = Instant::now();
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .map_err(|error| format!("native API request failed: {error}"))?;
    let status = response.status();
    if status.as_u16() != 200 {
        return Err(format!("native API returned HTTP {status}"));
    }
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|error| format!("native API response body failed: {error}"))?;
    let elapsed_ms = started.elapsed().as_millis();
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| format!("native API returned invalid JSON: {error}"))?;
    let total = payload
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "native API list response omitted its total".to_owned())?
        as usize;
    println!("scale uri={uri} elapsed_ms={elapsed_ms}");
    Ok((elapsed_ms, total))
}

async fn native_route_latency_ns(
    app: axum::Router,
    uri: &'static str,
    start: Arc<tokio::sync::Barrier>,
) -> Result<(&'static str, u128), String> {
    start.wait().await;
    let started = Instant::now();
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .map_err(|error| format!("native API request failed for {uri}: {error}"))?;
    let status = response.status();
    if status.as_u16() != 200 {
        return Err(format!("native API route {uri} returned HTTP {status}"));
    }
    let _body = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|error| format!("native API response body failed for {uri}: {error}"))?;
    Ok((uri, started.elapsed().as_nanos()))
}

async fn mixed_native_catalog_latency_ns(
    app: axum::Router,
    rounds: usize,
) -> Result<BTreeMap<&'static str, Vec<u128>>, String> {
    const ROUTES: [&str; 4] = [
        "/api/v1/torrents?limit=200&sort=name&reverse=true",
        "/api/v1/torrents?limit=200&offset=200",
        "/api/v1/transfer/info",
        "/metrics",
    ];
    let mut samples = ROUTES
        .into_iter()
        .map(|route| (route, Vec::with_capacity(rounds)))
        .collect::<BTreeMap<_, _>>();

    for _ in 0..rounds {
        let barrier = Arc::new(tokio::sync::Barrier::new(ROUTES.len()));
        let mut requests = JoinSet::new();
        for route in ROUTES {
            requests.spawn(native_route_latency_ns(
                app.clone(),
                route,
                Arc::clone(&barrier),
            ));
        }
        while let Some(result) = requests.join_next().await {
            let (route, elapsed_ns) =
                result.map_err(|error| format!("native API request task failed: {error}"))??;
            samples
                .get_mut(route)
                .ok_or_else(|| format!("unexpected native API route result: {route}"))?
                .push(elapsed_ns);
        }
    }
    Ok(samples)
}

fn percentile_ns(samples: &[u128], numerator: usize, denominator: usize) -> u128 {
    if samples.is_empty() || denominator == 0 || numerator == 0 || numerator > denominator {
        return 0;
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let rank = (ordered.len() * numerator).div_ceil(denominator).max(1);
    ordered[rank.saturating_sub(1).min(ordered.len() - 1)]
}

#[tokio::test]
async fn synthetic_registry_list_1k_proxy_under_100ms() {
    let app = torrentng_app_with(1_000).await;
    let ms = get_ms(app, "/api/v1/torrents").await;
    let limit = threshold(100);
    assert!(ms < limit, "1k list took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_registry_list_10k_proxy_under_200ms() {
    let app = torrentng_app_with(10_000).await;
    let ms = get_ms(app, "/api/v1/torrents").await;
    let limit = threshold(200);
    assert!(ms < limit, "10k list took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_registry_list_15k_proxy_under_500ms() {
    let app = torrentng_app_with(15_000).await;
    let ms = get_ms(app, "/api/v1/torrents").await;
    let limit = threshold(500);
    assert!(ms < limit, "15k list took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_database_seed_15k_proxy_under_120s() {
    let dataset = rt_testkit::SyntheticTorrentDataset::new(15_000);
    let conn = rt_testkit::memory_db().unwrap();

    let t0 = Instant::now();
    dataset.write_to_db(&conn).unwrap();
    let rows = rt_db::list_all(&conn).unwrap();
    let ms = t0.elapsed().as_millis();

    assert_eq!(rows.len(), 15_000);
    let limit = threshold(120_000);
    assert!(ms < limit, "15k cold DB load took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_registry_filter_sort_15k_proxy_under_250ms() {
    let app = torrentng_app_with(15_000).await;
    let ms = get_ms(
        app,
        "/api/v1/torrents?filter=all&sort=name&reverse=true&limit=200&offset=2000",
    )
    .await;
    let limit = threshold(250);
    assert!(
        ms < limit,
        "15k TorrentNG-client filter/sort took {ms}ms, want <{limit}ms"
    );
}

#[tokio::test]
async fn synthetic_qbit_info_50k_proxy_under_500ms() {
    let app = qbit_app_with(50_000).await;
    let ms = get_ms(app, "/api/qb/v2/torrents/info").await;
    let limit = threshold(500);
    assert!(ms < limit, "50k qbit/info took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_qbit_sync_maindata_15k_proxy_under_50ms() {
    let app = qbit_app_with(15_000).await;
    let ms = get_ms(app, "/api/qb/v2/sync/maindata").await;
    let limit = threshold(50);
    assert!(ms < limit, "sync/maindata 15k took {ms}ms, want <{limit}ms");
}

#[tokio::test]
async fn synthetic_qbit_idle_memory_15k_proxy_under_2_5gb() {
    let before = current_rss_bytes();
    let app = qbit_app_with(15_000).await;
    let _ = get_ms(app, "/api/qb/v2/torrents/info?limit=200").await;
    let after = current_rss_bytes();
    let rss = after.unwrap_or(before.unwrap_or(0));
    let limit = 2_500_u64 * 1024 * 1024;

    assert!(rss < limit, "15k idle RSS is {rss} bytes, want <{limit}");
}

#[tokio::test]
async fn synthetic_qbit_idle_memory_100k_proxy_keeps_fixed_task_fd_budget() {
    // This reads process-wide /proc/self/task counts. CI serializes tests within
    // each binary so unrelated test-harness threads do not consume the budget.
    let before_rss = current_rss_bytes();
    let before_fds = current_fd_count();
    let before_tasks = current_task_count();
    let app = qbit_app_with(100_000).await;
    let _ = get_ms(app, "/api/qb/v2/torrents/info?limit=200").await;
    let after_rss = current_rss_bytes();
    let after_fds = current_fd_count();
    let after_tasks = current_task_count();
    let rss = after_rss.unwrap_or(before_rss.unwrap_or(0));
    println!(
        "scale 100k_idle rss_bytes={rss} fds_before={before_fds:?} fds_after={after_fds:?} tasks_before={before_tasks:?} tasks_after={after_tasks:?}"
    );
    let limit = 2_500_u64 * 1024 * 1024;

    assert!(rss < limit, "100k idle RSS is {rss} bytes, want <{limit}");
    if let (Some(before), Some(after)) = (before_fds, after_fds) {
        assert!(
            after <= before + 64,
            "100k idle fd count grew from {before} to {after}"
        );
    }
    if let (Some(before), Some(after)) = (before_tasks, after_tasks) {
        assert!(
            after <= before + 8,
            "100k idle task count grew from {before} to {after}"
        );
    }
}

struct LiveEngineCatalog {
    _temp: tempfile::TempDir,
    engine: rt_engine::EngineHandle,
    registry: Arc<RwLock<SessionRegistry>>,
    router: axum::Router,
    fixture_generation_ns: u128,
    engine_admission_ns: u128,
    restore_ns: u128,
}

fn live_catalog_memory_caps_mb(count: usize) -> (u64, u64) {
    const MIB: u64 = 1024 * 1024;
    const METADATA_PER_TORRENT_BUDGET: u64 = 128 * 1024;
    const MIN_METADATA_CAP_MB: u64 = 128;
    const MIN_TOTAL_HEADROOM_MB: u64 = 512;

    let count = u64::try_from(count).unwrap_or(u64::MAX);
    let metadata_bytes = count.saturating_mul(METADATA_PER_TORRENT_BUDGET);
    let metadata_cap_mb = metadata_bytes.div_ceil(MIB).max(MIN_METADATA_CAP_MB);
    let total_headroom_mb = (metadata_cap_mb / 2).max(MIN_TOTAL_HEADROOM_MB);
    let total_cap_mb = metadata_cap_mb.saturating_add(total_headroom_mb);
    (metadata_cap_mb, total_cap_mb)
}

#[test]
fn live_catalog_memory_caps_scale_with_catalog_size() {
    assert_eq!(live_catalog_memory_caps_mb(1_000), (128, 640));
    assert_eq!(live_catalog_memory_caps_mb(100_000), (12_500, 18_750));
}

impl LiveEngineCatalog {
    async fn build(count: usize) -> Result<Self, String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let download_dir = temp.path().join("downloads");
        std::fs::create_dir_all(&download_dir).map_err(|error| error.to_string())?;

        let mut config = Config::default();
        // Size metadata and API-snapshot headroom for the catalog query. The
        // paused rows stay dormant and do not retain parsed metainfo or piece
        // indexes; the 100k query still needs room for its live API projection.
        let (metadata_cap_mb, total_cap_mb) = live_catalog_memory_caps_mb(count);
        config.memory.metadata_cap_mb = metadata_cap_mb;
        config.memory.total_cap_mb = total_cap_mb;
        config.daemon.session_dir = temp.path().join("session");
        config.daemon.shutdown_timeout_secs = 30;
        config.db.path = temp.path().join("state.sqlite");
        config.storage.download_dir = download_dir.clone();
        config.network.listen_port = 0;
        config.dht.enabled = false;

        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        let ingest_engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
            .await
            .map_err(|error| error.to_string())?;

        let mut fixture_generation_ns = 0_u128;
        let mut engine_admission_ns = 0_u128;
        for index in 0..count {
            let fixture_started = Instant::now();
            let metainfo = rt_testkit::production_catalog_metainfo(index);
            fixture_generation_ns =
                fixture_generation_ns.saturating_add(fixture_started.elapsed().as_nanos());
            let admission_started = Instant::now();
            let result = ingest_engine
                .add_torrent_raw_with_labels(
                    metainfo,
                    Some(download_dir.clone()),
                    true,
                    None,
                    Vec::new(),
                )
                .await;
            engine_admission_ns =
                engine_admission_ns.saturating_add(admission_started.elapsed().as_nanos());
            if let Err(error) = result {
                ingest_engine.shutdown().await;
                return Err(format!("engine rejected catalog fixture {index}: {error}"));
            }
        }

        let (registered, all_paused) = {
            let registry = registry.read().await;
            let snapshot = (
                registry.len(),
                registry
                    .iter()
                    .all(|torrent| torrent.state == TorrentState::Paused),
            );
            snapshot
        };
        if registered != count {
            ingest_engine.shutdown().await;
            return Err(format!(
                "engine registered {registered} of {count} catalog fixtures"
            ));
        }
        if !all_paused {
            ingest_engine.shutdown().await;
            return Err("catalog fixture unexpectedly started a torrent".to_owned());
        }

        ingest_engine.shutdown().await;
        let restored_registry = Arc::new(RwLock::new(SessionRegistry::new()));
        let restore_started = Instant::now();
        let engine = Engine::start(Arc::new(config), Arc::clone(&restored_registry))
            .await
            .map_err(|error| error.to_string())?;
        let restore_ns = restore_started.elapsed().as_nanos();
        let (restored_count, restored_all_paused) = {
            let registry = restored_registry.read().await;
            let snapshot = (
                registry.len(),
                registry
                    .iter()
                    .all(|torrent| torrent.state == TorrentState::Paused),
            );
            snapshot
        };
        if restored_count != count {
            engine.shutdown().await;
            return Err(format!(
                "engine restored {restored_count} of {count} catalog fixtures"
            ));
        }
        if !restored_all_paused {
            engine.shutdown().await;
            return Err("restored catalog fixture unexpectedly started a torrent".to_owned());
        }

        let router = build_router(AppState::with_engine(
            Arc::clone(&restored_registry),
            engine.clone(),
        ));
        Ok(Self {
            _temp: temp,
            engine,
            registry: restored_registry,
            router,
            fixture_generation_ns,
            engine_admission_ns,
            restore_ns,
        })
    }
}

async fn observe_live_native_engine_catalog(count: usize) {
    let fixture = LiveEngineCatalog::build(count)
        .await
        .unwrap_or_else(|error| panic!("live native catalog setup failed: {error}"));
    let registered = fixture.registry.read().await.len();
    let query = "/api/v1/torrents?limit=200&sort=name&reverse=true";
    let first_list = native_list_ms_and_total(fixture.router.clone(), query).await;
    let cached_list = native_list_ms_and_total(fixture.router.clone(), query).await;
    fixture.engine.shutdown().await;
    let (first_list_ms, first_total) =
        first_list.unwrap_or_else(|error| panic!("first native list observation failed: {error}"));
    let (cached_list_ms, cached_total) = cached_list
        .unwrap_or_else(|error| panic!("cached native list observation failed: {error}"));
    assert_eq!(first_total, count, "first native list total");
    assert_eq!(cached_total, count, "cached native list total");
    println!(
        "scale live_native_engine_catalog count={registered} valid_metainfo=true paused=true fixture_generation_ns={} engine_admission_ns={} restore_ns={} first_list_ms={first_list_ms} cached_list_ms={cached_list_ms}",
        fixture.fixture_generation_ns,
        fixture.engine_admission_ns,
        fixture.restore_ns,
    );
}

async fn observe_live_native_catalog_mixed_load(count: usize) {
    let fixture = LiveEngineCatalog::build(count)
        .await
        .unwrap_or_else(|error| panic!("live native catalog setup failed: {error}"));
    let samples = mixed_native_catalog_latency_ns(fixture.router.clone(), 16)
        .await
        .unwrap_or_else(|error| panic!("live native mixed-load observation failed: {error}"));
    let mut route_records = Vec::with_capacity(samples.len());
    for (route, route_samples) in &samples {
        assert_eq!(route_samples.len(), 16, "sample count for {route}");
        let p50_ns = percentile_ns(route_samples, 50, 100);
        let p95_ns = percentile_ns(route_samples, 95, 100);
        println!(
            "scale live_native_catalog_mixed count={count} route={route} samples={} p50_ns={p50_ns} p95_ns={p95_ns}",
            route_samples.len(),
        );
        route_records.push(serde_json::json!({
            "route": route,
            "sample_count": route_samples.len(),
            "p50_ns": p50_ns,
            "p95_ns": p95_ns,
            "samples_ns": route_samples,
        }));
    }
    println!(
        "scale_live_native_catalog_mixed_json={}",
        serde_json::json!({
            "schema_version": "transfer-live-catalog-mixed-v1",
            "catalog_rows": count,
            "catalog_state": "restored_paused",
            "rounds": 16,
            "concurrent_routes_per_round": 4,
            "routes": route_records,
        })
    );
    fixture.engine.shutdown().await;
}

#[tokio::test]
async fn live_native_engine_catalog_1k_uses_valid_persisted_metainfo() {
    observe_live_native_engine_catalog(1_000).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_native_engine_catalog_scrape_mixed_load_1k() {
    observe_live_native_catalog_mixed_load(1_000).await;
}

#[tokio::test]
#[ignore = "100k live-engine catalog qualification belongs in the bulk scale pass"]
async fn live_native_engine_catalog_100k_uses_valid_persisted_metainfo() {
    observe_live_native_engine_catalog(100_000).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "100k live-engine mixed-load qualification belongs in the bulk scale pass"]
async fn live_native_engine_catalog_scrape_mixed_load_100k() {
    observe_live_native_catalog_mixed_load(100_000).await;
}

#[test]
fn tracker_restart_storm_15k_is_spread_by_jitter() {
    let interval = Duration::from_secs(30 * 60);
    let mut buckets = std::collections::BTreeMap::<u64, usize>::new();
    let mut min = u64::MAX;
    let mut max = 0;
    for _ in 0..15_000 {
        let seconds = jitter_interval(interval, 0.2).as_secs();
        min = min.min(seconds);
        max = max.max(seconds);
        *buckets.entry(seconds / 10).or_default() += 1;
    }

    let busiest_ten_second_bucket = buckets.values().copied().max().unwrap_or_default();
    assert!(min >= 1_440, "minimum jittered interval {min}s is too low");
    assert!(max <= 2_160, "maximum jittered interval {max}s is too high");
    assert!(
        max - min > 300,
        "jitter spread {}s is too narrow",
        max - min
    );
    assert!(
        busiest_ten_second_bucket < 750,
        "restart storm bucket too dense: {busiest_ten_second_bucket} announces in 10s"
    );
}

#[tokio::test]
async fn recheck_does_not_starve_seeding_peer_reads() {
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 256,
            recheck_concurrency: 1,
            peer_read_concurrency: 4,
            ..Default::default()
        },
    );
    let _recheck = scheduler.acquire(IoClass::Recheck).await.unwrap();

    let mut peer_read_permits = Vec::new();
    for _ in 0..4 {
        peer_read_permits.push(
            scheduler
                .try_acquire(IoClass::PeerRead)
                .expect("peer reads must not wait behind recheck permits"),
        );
    }

    assert_eq!(scheduler.available_permits(IoClass::Recheck), 0);
    assert_eq!(scheduler.available_permits(IoClass::PeerRead), 0);
}

#[tokio::test]
async fn storage_file_pool_stays_bounded_under_active_file_churn() {
    let dir = tempfile::tempdir().unwrap();
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Hdd,
            storage_io: StorageIoConfig {
                file_pool_size: 8,
                io_worker_threads: 4,
                io_queue_depth: 64,
                ..Default::default()
            },
            ..Default::default()
        },
    );

    for i in 0..64 {
        let path = dir.path().join(format!("payload-{i}.bin"));
        std::fs::write(&path, vec![i as u8; 4096]).unwrap();
        let data = scheduler
            .read_at(IoClass::PeerRead, &path, 0, 1024)
            .await
            .unwrap();
        assert_eq!(data.len(), 1024);
    }

    let stats = scheduler.stats();
    assert_eq!(stats.file_pool.capacity, 8);
    assert!(
        stats.file_pool.open_files <= 8,
        "file pool leaked descriptors: {:?}",
        stats.file_pool
    );
    assert!(
        stats.file_pool.evictions >= 56,
        "expected LRU churn under capacity pressure: {:?}",
        stats.file_pool
    );
    assert_eq!(
        stats.read_ops_by_class[IoClass::PeerRead as usize],
        64,
        "every peer read should be accounted by class"
    );
}

#[tokio::test]
async fn storage_peer_read_readahead_reduces_backend_reads_for_adjacent_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie-piece.bin");
    let payload: Vec<u8> = (0..512 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &payload).unwrap();
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Hdd,
            storage_io: StorageIoConfig {
                peer_read_readahead_bytes: 512 * 1024,
                ..Default::default()
            },
            ..Default::default()
        },
    );

    for block in 0..32 {
        let offset = block * 16 * 1024;
        let data = scheduler
            .read_at(IoClass::PeerRead, &path, offset as u64, 16 * 1024)
            .await
            .unwrap();
        assert_eq!(&data[..], &payload[offset..offset + 16 * 1024]);
    }

    let stats = scheduler.stats();
    assert!(stats.peer_read_cache_misses < 32);
    assert!(stats.peer_read_cache_hits > 0);
    assert_eq!(
        stats.peer_read_cache_misses + stats.peer_read_cache_hits,
        32,
        "every logical read should be classified as a cache hit or miss"
    );
    assert!(
        (1..=64).contains(&stats.peer_read_cache_entries),
        "sequential reads should retain a bounded set of reusable cache ranges: {:?}",
        stats.peer_read_cache_entries
    );
    assert_eq!(
        stats.read_ops_by_class[IoClass::PeerRead as usize],
        32,
        "logical peer reads should still be accounted individually"
    );
    assert!(
        stats.backend_read_ops_by_class[IoClass::PeerRead as usize] < 32,
        "adaptive readahead should reduce backend reads after sequential cache hits"
    );
    let backend_bytes = stats.backend_bytes_read_by_class[IoClass::PeerRead as usize];
    assert!(backend_bytes >= payload.len() as u64);
    assert!(
        backend_bytes <= payload.len() as u64 + 16 * 1024,
        "sequential readahead should stay within one block of useful bytes: {backend_bytes}"
    );
}

#[tokio::test]
async fn storage_positioned_io_preserves_offsets_under_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("positioned-payload.bin");
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Ssd,
            storage_io: StorageIoConfig {
                io_worker_threads: 8,
                io_queue_depth: 128,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    scheduler
        .prepare_file(&path, 128 * 4096, PreallocationMode::Sparse)
        .await
        .unwrap();

    let mut writes = Vec::new();
    for block in 0..128 {
        let scheduler = scheduler.clone();
        let path = path.clone();
        writes.push(tokio::spawn(async move {
            let fill = (block % 251) as u8;
            scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &path,
                    block * 4096,
                    bytes::Bytes::from(vec![fill; 4096]),
                    false,
                )
                .await
                .unwrap();
        }));
    }
    for write in writes {
        write.await.unwrap();
    }

    for block in [0_u64, 1, 31, 64, 127] {
        let data = scheduler
            .read_at(IoClass::Foreground, &path, block * 4096, 4096)
            .await
            .unwrap();
        assert!(data.iter().all(|byte| *byte == (block % 251) as u8));
    }

    let stats = scheduler.stats();
    assert_eq!(stats.write_ops_by_class[IoClass::PeerWrite as usize], 128);
    assert_eq!(
        stats.bytes_written_by_class[IoClass::PeerWrite as usize],
        128 * 4096
    );
}

#[tokio::test]
async fn storage_hash_pool_does_not_block_peer_read_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seed.bin");
    std::fs::write(&path, vec![7_u8; 64 * 1024]).unwrap();
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Hdd,
            storage_io: StorageIoConfig {
                hash_worker_threads: 1,
                hash_queue_depth: 2,
                io_worker_threads: 2,
                io_queue_depth: 16,
                ..Default::default()
            },
            ..Default::default()
        },
    );

    let mut hash_tasks = Vec::new();
    for i in 0..8 {
        let scheduler = scheduler.clone();
        hash_tasks.push(tokio::spawn(async move {
            let data = bytes::Bytes::from(vec![i as u8; 1024 * 1024]);
            scheduler.hash_sha1(data).await
        }));
    }

    let read = tokio::time::timeout(
        Duration::from_millis(threshold(50) as u64),
        read_at_retry_queue_full(&scheduler, IoClass::PeerRead, &path, 0, 16 * 1024),
    )
    .await
    .expect("peer read should not wait behind hash queue")
    .unwrap();
    assert_eq!(read.len(), 16 * 1024);

    let mut hash_successes = 0;
    let mut hash_backpressure = 0;
    for task in hash_tasks {
        match task.await.unwrap() {
            Ok(hash) => {
                hash_successes += 1;
                assert_ne!(hash, [0; 20]);
            }
            Err(StorageError::QueueFull { .. }) => {
                hash_backpressure += 1;
            }
            Err(e) => panic!("unexpected hash error: {e}"),
        }
    }
    assert!(hash_successes > 0);
    // Queue-full is timing-dependent once optimized hashing completes faster
    // than the async tasks can submit. The invariant under test is that peer
    // reads remain available either way; deterministic queue rejection is
    // covered by rt-storage's BlockingPool tests.

    let stats = scheduler.stats();
    assert_eq!(stats.hash_ops, hash_successes);
    assert!(stats.queue_full >= hash_backpressure);
    assert_eq!(stats.read_ops_by_class[IoClass::PeerRead as usize], 1);
}

#[tokio::test]
async fn storage_recheck_hashing_reports_scheduler_result_without_runtime_stall() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recheck.bin");
    let content = b"piece-zero-piece-one";
    std::fs::write(&path, content).unwrap();
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Hdd,
            recheck_concurrency: 1,
            peer_read_concurrency: 4,
            ..Default::default()
        },
    );
    let hash = hash_sha1_retry_queue_full(&scheduler, bytes::Bytes::copy_from_slice(content))
        .await
        .unwrap();
    let piece_map = PieceMap::new(
        content.len() as u64,
        vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("recheck.bin", false).unwrap(),
            content_offset: 0,
            length: content.len() as u64,
        }],
    )
    .unwrap();
    let hashes = [hash];
    let verifier = rt_storage::PieceVerifier::new(dir.path(), &scheduler, &piece_map, &hashes);

    let result = tokio::time::timeout(Duration::from_secs(2), verifier.verify_piece(0))
        .await
        .expect("recheck hashing should finish without stalling the runtime");
    assert_eq!(result, VerifyResult::Valid);

    let stats = scheduler.stats();
    assert_eq!(stats.read_ops_by_class[IoClass::Recheck as usize], 1);
    assert!(stats.hash_ops >= 2);
}

#[tokio::test]
async fn completed_piece_ram_hash_avoids_read_after_write_backend_reads() {
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Ssd,
            ..Default::default()
        },
    );
    let piece = bytes::Bytes::from(vec![0x42u8; 1024 * 1024]);
    let hash = hash_sha1_retry_queue_full(&scheduler, piece.clone())
        .await
        .unwrap();
    let before = scheduler.stats();
    let verified = hash_sha1_retry_queue_full(&scheduler, piece).await.unwrap();
    let after = scheduler.stats();

    assert_eq!(verified, hash);
    let before_backend_reads = before.backend_read_ops_by_class.iter().sum::<u64>();
    let after_backend_reads = after.backend_read_ops_by_class.iter().sum::<u64>();
    assert_eq!(
        after_backend_reads - before_backend_reads,
        0,
        "RAM piece verification must not perform read-after-write disk I/O"
    );
    assert_eq!(after.hash_ops - before.hash_ops, 1);
}

#[test]
fn crash_watermark_bounds_restart_recheck_to_dirty_pieces() {
    let piece_count = 100_000u32;
    let dirty: Vec<u32> = (0..512).map(|piece| piece * 3).collect();
    let mut state =
        FastresumeState::new_empty(&[9u8; 20], piece_count, ImportPolicy::RequireVerification);
    state.pieces.fill(PieceState::Valid);
    state.clean_shutdown = false;
    state.set_dirty_pieces_since_barrier(dirty.iter().copied());

    let downgraded = state
        .apply_unclean_shutdown_watermark()
        .expect("watermark should bound unclean restart recheck");

    assert_eq!(downgraded, dirty.len() as u32);
    assert_eq!(state.unknown_piece_count(), dirty.len() as u32);
    assert_eq!(
        state.valid_piece_count(),
        piece_count - dirty.len() as u32,
        "restart should not force a full-library recheck"
    );
    assert!(state.clean_shutdown);
    assert!(state.durability.dirty_pieces_since_barrier.is_empty());
}

#[tokio::test]
async fn sparse_recheck_skips_holes_and_reports_extent_counters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse-scale.bin");
    let file_len = 4 * 1024 * 1024usize;
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(file_len as u64).unwrap();
    drop(file);
    let data = vec![0x33u8; 16 * 1024];
    let data_offset = 2 * 1024 * 1024usize;
    std::fs::write(dir.path().join("payload.tmp"), &data).unwrap();
    let scheduler = MountScheduler::new(
        StorageRootId::new(),
        &SchedulerConfig {
            profile: StorageProfile::Ssd,
            ..Default::default()
        },
    );
    scheduler
        .write_at(
            IoClass::PeerWrite,
            &path,
            data_offset as u64,
            bytes::Bytes::from(data.clone()),
            false,
        )
        .await
        .unwrap();

    let mut expected = vec![0u8; file_len];
    expected[data_offset..data_offset + data.len()].copy_from_slice(&data);
    let expected_hash = hash_sha1_retry_queue_full(&scheduler, bytes::Bytes::from(expected))
        .await
        .unwrap();
    let piece_map = PieceMap::new(
        file_len as u64,
        vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("sparse-scale.bin", false).unwrap(),
            content_offset: 0,
            length: file_len as u64,
        }],
    )
    .unwrap();
    let hashes = [expected_hash];
    let verifier = rt_storage::PieceVerifier::new(dir.path(), &scheduler, &piece_map, &hashes);

    assert_eq!(verifier.verify_piece(0).await, VerifyResult::Valid);
    let stats = scheduler.stats();
    assert!(
        stats.sparse_data_extents > 0 || stats.sparse_seek_fallbacks > 0,
        "sparse recheck should either map extents or record an unsupported-filesystem fallback"
    );
}

fn current_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb = line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())?;
    Some(kb * 1024)
}

fn current_fd_count() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count())
}

fn current_task_count() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/task").ok()?.count())
}
