//! Full-engine protocol and restart invariants for production-shaped fixtures.
//!
//! These cases exercise metainfo admission, disk recheck, persisted completion
//! state, clean restart, and the recheck performed when a dormant torrent is
//! resumed. They are correctness gates, not transfer-rate measurements.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rt_config::Config;
use rt_engine::{Engine, EngineHandle};
use rt_session::{SessionRegistry, TorrentState};
use rt_testkit::{
    public_transfer_compatibility_matrix, TransferCompatibilityFixture, TransferProtocol,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

struct ProtocolCase {
    protocol: TransferProtocol,
    info_hash: String,
    save_path: PathBuf,
    fixture: TransferCompatibilityFixture,
}

fn protocol_label(protocol: TransferProtocol) -> &'static str {
    match protocol {
        TransferProtocol::V1 => "v1",
        TransferProtocol::V2 => "v2",
        TransferProtocol::Hybrid => "hybrid",
    }
}

fn payload_root(save_path: &Path, fixture: &TransferCompatibilityFixture) -> PathBuf {
    match fixture.protocol {
        TransferProtocol::V2 => save_path.to_path_buf(),
        TransferProtocol::V1 | TransferProtocol::Hybrid => save_path.join("transfer-compatibility"),
    }
}

fn materialize_fixture(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let torrent_root = payload_root(save_path, fixture);
    for file in &fixture.payload_files {
        let mut path = torrent_root.clone();
        for component in &file.path {
            path.push(*component);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &file.bytes)?;
    }

    if fixture.protocol == TransferProtocol::Hybrid {
        let first_file_bytes = fixture
            .payload_files
            .first()
            .map_or(0, |file| file.bytes.len());
        let padding_bytes =
            (fixture.piece_length - first_file_bytes % fixture.piece_length) % fixture.piece_length;
        if padding_bytes != 0 {
            let pad_path = torrent_root.join(".pad").join(padding_bytes.to_string());
            std::fs::create_dir_all(pad_path.parent().expect("pad path has a parent"))?;
            std::fs::write(pad_path, vec![0; padding_bytes])?;
        }
    }

    Ok(())
}

fn prepare_incomplete_fixture(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let torrent_root = payload_root(save_path, fixture);
    std::fs::create_dir_all(&torrent_root)?;
    for file in &fixture.payload_files {
        if !file.bytes.is_empty() {
            continue;
        }
        let mut path = torrent_root.clone();
        for component in &file.path {
            path.push(*component);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &file.bytes)?;
    }
    if fixture.protocol == TransferProtocol::Hybrid {
        let first_file_bytes = fixture
            .payload_files
            .first()
            .map_or(0, |file| file.bytes.len());
        let padding_bytes =
            (fixture.piece_length - first_file_bytes % fixture.piece_length) % fixture.piece_length;
        if padding_bytes != 0 {
            let pad_path = torrent_root.join(".pad").join(padding_bytes.to_string());
            std::fs::create_dir_all(pad_path.parent().expect("pad path has a parent"))?;
            std::fs::write(pad_path, vec![0; padding_bytes])?;
        }
    }
    Ok(())
}

fn unused_tcp_port() -> u16 {
    StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .expect("reserve a loopback port")
        .port()
}

struct PeerIdRewritingProxy {
    addr: SocketAddr,
    accept_task: JoinHandle<()>,
}

impl Drop for PeerIdRewritingProxy {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn start_peer_id_rewriting_proxy(target: SocketAddr) -> PeerIdRewritingProxy {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind peer identity proxy");
    let addr = listener.local_addr().expect("read proxy address");
    let accept_task = tokio::spawn(async move {
        while let Ok((mut downstream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut upstream = match TcpStream::connect(target).await {
                    Ok(upstream) => upstream,
                    Err(_) => return,
                };
                let mut downstream_handshake = [0; 68];
                let mut upstream_handshake = [0; 68];
                if downstream
                    .read_exact(&mut downstream_handshake)
                    .await
                    .is_err()
                {
                    return;
                }

                // Each Engine in this process shares the installation peer ID.
                // Give the two test engines distinct wire identities so the
                // real self-connection guard stays enabled during transfer.
                downstream_handshake[48..68].fill(b'P');
                if upstream.write_all(&downstream_handshake).await.is_err() {
                    return;
                }
                if upstream.read_exact(&mut upstream_handshake).await.is_err() {
                    return;
                }
                upstream_handshake[48..68].fill(b'Q');
                if downstream.write_all(&upstream_handshake).await.is_err() {
                    return;
                }

                let (mut downstream_read, mut downstream_write) = downstream.into_split();
                let (mut upstream_read, mut upstream_write) = upstream.into_split();
                let _ = tokio::try_join!(
                    tokio::io::copy(&mut downstream_read, &mut upstream_write),
                    tokio::io::copy(&mut upstream_read, &mut downstream_write)
                );
            });
        }
    });
    PeerIdRewritingProxy { addr, accept_task }
}

fn assert_fixture_contents(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let torrent_root = payload_root(save_path, fixture);
    for file in &fixture.payload_files {
        let mut path = torrent_root.clone();
        for component in &file.path {
            path.push(*component);
        }
        let actual = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "payload file {:?} is missing at {}: {error}",
                file.path,
                path.display()
            )
        });
        assert_eq!(actual, file.bytes, "payload file {:?}", file.path);
    }
    if fixture.protocol == TransferProtocol::Hybrid {
        let first_file_bytes = fixture
            .payload_files
            .first()
            .map_or(0, |file| file.bytes.len());
        let padding_bytes =
            (fixture.piece_length - first_file_bytes % fixture.piece_length) % fixture.piece_length;
        if padding_bytes != 0 {
            let path = torrent_root.join(".pad").join(padding_bytes.to_string());
            assert_eq!(std::fs::read(path)?, vec![0; padding_bytes]);
        }
    }
    Ok(())
}

fn remove_fixture_root(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let root = payload_root(save_path, fixture);
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

async fn wait_for_state(
    engine: &EngineHandle,
    registry: &Arc<RwLock<SessionRegistry>>,
    info_hash: &str,
    expected: TorrentState,
    context: &str,
) -> u64 {
    let mut last_observed = None;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let observed = {
                let registry = registry.read().await;
                registry.get(info_hash).map(|entry| {
                    (
                        entry.state,
                        entry.amount_left,
                        entry.stats.downloaded,
                        entry.tracker_message.clone(),
                        entry.error_message.clone(),
                    )
                })
            };
            last_observed = observed.clone();
            match observed {
                Some((state, amount_left, _, _, _)) if state == expected => return amount_left,
                Some((TorrentState::Error, _, _, _, error)) => {
                    panic!("{context} torrent {info_hash} entered Error: {error:?}")
                }
                Some(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                None => panic!("torrent {info_hash} disappeared while waiting for {expected:?}"),
            }
        }
    })
    .await;
    match result {
        Ok(amount_left) => amount_left,
        Err(_) => {
            let peers = engine.torrent_peers(info_hash.to_owned()).await;
            panic!("{context} torrent {info_hash} did not reach {expected:?}; last state/left/downloaded/tracker/error={last_observed:?}; peer snapshots={peers:?}")
        }
    }
}

fn test_config(root: &Path, listen_port: u16) -> Config {
    let mut config = Config::default();
    config.daemon.session_dir = root.join("session");
    config.daemon.shutdown_timeout_secs = 30;
    config.db.path = root.join("state.sqlite");
    config.storage.download_dir = root.join("downloads");
    config.network.listen_port = listen_port;
    config.dht.enabled = false;
    config.runtime.torrent_tiers_enabled = false;
    config.tracker.allow_loopback_egress = true;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persisted_v2_completion_is_rechecked_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let seed_config = test_config(&temp.path().join("seed"), unused_tcp_port());
    let receive_config = test_config(&temp.path().join("receive"), unused_tcp_port());
    let seed_download_dir = seed_config.storage.download_dir.clone();
    let receive_download_dir = receive_config.storage.download_dir.clone();
    std::fs::create_dir_all(&seed_download_dir).unwrap();
    std::fs::create_dir_all(&receive_download_dir).unwrap();

    let seed_registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let seed_engine = Engine::start(Arc::new(seed_config), Arc::clone(&seed_registry))
        .await
        .expect("seed engine start");
    let receive_registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let receive_engine = Engine::start(
        Arc::new(receive_config.clone()),
        Arc::clone(&receive_registry),
    )
    .await
    .expect("receive engine start");
    let mut cases = Vec::new();

    // The fixture's placeholder tracker uses a deliberately unresolvable
    // domain. Clear it so this test exercises only the injected local peer.
    for fixture in public_transfer_compatibility_matrix() {
        let seed_save_path = seed_download_dir.join(protocol_label(fixture.protocol));
        materialize_fixture(&seed_save_path, &fixture).expect("materialize seed payload");
        let info_hash = seed_engine
            .add_torrent_raw_with_labels(
                fixture.torrent.clone(),
                Some(seed_save_path),
                true,
                None,
                Vec::new(),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("engine rejected {:?} metainfo: {error}", fixture.protocol)
            });
        seed_engine
            .update_torrent_trackers(info_hash.clone(), Vec::new())
            .await
            .unwrap_or_else(|error| {
                panic!("cannot clear fixture tracker for {info_hash}: {error}")
            });
        let save_path = receive_download_dir.join(protocol_label(fixture.protocol));
        prepare_incomplete_fixture(&save_path, &fixture).expect("prepare empty receiver save path");
        let receive_hash = receive_engine
            .add_torrent_raw_with_labels(
                fixture.torrent.clone(),
                Some(save_path.clone()),
                true,
                None,
                Vec::new(),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("receiver rejected {:?} metainfo: {error}", fixture.protocol)
            });
        assert_eq!(receive_hash, info_hash, "protocol info hash is stable");
        receive_engine
            .update_torrent_trackers(receive_hash.clone(), Vec::new())
            .await
            .unwrap_or_else(|error| {
                panic!("cannot clear receiver fixture tracker for {receive_hash}: {error}")
            });
        cases.push(ProtocolCase {
            protocol: fixture.protocol,
            info_hash: receive_hash,
            save_path,
            fixture,
        });
    }

    for case in &cases {
        seed_engine
            .resume_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot start {:?} seed: {error}", case.protocol));
        let amount_left = wait_for_state(
            &seed_engine,
            &seed_registry,
            &case.info_hash,
            TorrentState::Seeding,
            &format!("{:?} seed", case.protocol),
        )
        .await;
        assert_eq!(amount_left, 0, "{:?} seed is complete", case.protocol);
    }
    let seed_peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        seed_engine.listen_port().await.unwrap(),
    ));
    let peer_proxy = start_peer_id_rewriting_proxy(seed_peer).await;
    for case in &cases {
        receive_engine
            .resume_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot start {:?} receiver: {error}", case.protocol));
        wait_for_state(
            &receive_engine,
            &receive_registry,
            &case.info_hash,
            TorrentState::Downloading,
            &format!("{:?} receiver", case.protocol),
        )
        .await;
        // These public fixtures isolate peer-transfer and restart behavior.
        // Private peer authorization is covered by its own policy regressions;
        // initiating from both ends here would create duplicate loopback dials.
        receive_engine
            .add_peers(case.info_hash.clone(), vec![peer_proxy.addr])
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "cannot connect {:?} receiver to seed: {error}",
                    case.protocol
                )
            });
        let amount_left = wait_for_state(
            &receive_engine,
            &receive_registry,
            &case.info_hash,
            TorrentState::Seeding,
            &format!("{:?} transfer", case.protocol),
        )
        .await;
        assert_eq!(amount_left, 0, "{:?} transfer is complete", case.protocol);
        assert_fixture_contents(&case.save_path, &case.fixture).unwrap_or_else(|error| {
            panic!("{:?} downloaded payload is invalid: {error}", case.protocol)
        });

        if case.protocol == TransferProtocol::V2 {
            // Exercise the rollback actor as a live payload writer as well as
            // a restart candidate. Removing its complete save tree forces the
            // reference route to prepare missing parent directories and files
            // before its first streamed block write.
            receive_engine
                .set_transfer_owner(
                    case.info_hash.clone(),
                    rt_engine::TransferActorOwner::Reference,
                )
                .await
                .expect("switch pure-v2 torrent to reference actor");
            receive_engine
                .pause_torrent(case.info_hash.clone())
                .await
                .expect("pause reference pure-v2 receiver");
            remove_fixture_root(&case.save_path, &case.fixture)
                .expect("remove pure-v2 receiver save tree");
            receive_engine
                .resume_torrent(case.info_hash.clone())
                .await
                .expect("resume reference pure-v2 receiver");
            wait_for_state(
                &receive_engine,
                &receive_registry,
                &case.info_hash,
                TorrentState::Downloading,
                "reference pure-v2 receiver after payload removal",
            )
            .await;
            receive_engine
                .add_peers(case.info_hash.clone(), vec![peer_proxy.addr])
                .await
                .expect("connect reference pure-v2 receiver to seed");
            let amount_left = wait_for_state(
                &receive_engine,
                &receive_registry,
                &case.info_hash,
                TorrentState::Seeding,
                "reference pure-v2 transfer",
            )
            .await;
            assert_eq!(
                amount_left,
                0,
                "reference pure-v2 transfer is complete; final entry={:?}",
                receive_registry
                    .read()
                    .await
                    .get(&case.info_hash)
                    .map(|entry| (
                        entry.state,
                        entry.amount_left,
                        entry.finalizing,
                        entry.completed_at,
                        entry.stats.downloaded,
                    )),
            );
            assert_fixture_contents(&case.save_path, &case.fixture)
                .expect("reference pure-v2 payload is valid");
            receive_engine
                .set_transfer_owner(
                    case.info_hash.clone(),
                    rt_engine::TransferActorOwner::SharedCore,
                )
                .await
                .expect("restore pure-v2 torrent to shared actor");
            wait_for_state(
                &receive_engine,
                &receive_registry,
                &case.info_hash,
                TorrentState::Seeding,
                "shared pure-v2 actor after reference cutover",
            )
            .await;
        }
    }
    for case in &cases {
        seed_engine
            .pause_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot pause {:?} seed: {error}", case.protocol));
        receive_engine
            .pause_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot pause {:?} receiver: {error}", case.protocol));
        let registry = receive_registry.read().await;
        let paused = registry
            .get(&case.info_hash)
            .unwrap_or_else(|| panic!("paused {:?} receiver disappeared", case.protocol));
        assert_eq!(
            paused.state,
            TorrentState::Paused,
            "{:?} receiver paused",
            case.protocol
        );
        assert_eq!(
            paused.amount_left, 0,
            "{:?} complete at pause",
            case.protocol
        );
    }
    seed_engine.shutdown().await;
    receive_engine.shutdown().await;

    {
        let registry = receive_registry.read().await;
        for case in &cases {
            let stopped = registry
                .get(&case.info_hash)
                .unwrap_or_else(|| panic!("stopped {:?} receiver disappeared", case.protocol));
            assert_eq!(
                stopped.amount_left, 0,
                "{:?} complete after shutdown",
                case.protocol
            );
        }
    }
    let persisted_db = rusqlite::Connection::open(&receive_config.db.path)
        .expect("open receiver database after engine shutdown");
    for case in &cases {
        let persisted_amount_left: i64 = persisted_db
            .query_row(
                "SELECT amount_left FROM torrents WHERE info_hash = ?1",
                [case.info_hash.as_str()],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "read persisted {:?} completion state: {error}",
                    case.protocol
                )
            });
        assert_eq!(
            persisted_amount_left, 0,
            "{:?} completion must be persisted before payload damage",
            case.protocol
        );
    }
    drop(persisted_db);

    let v2_case = cases
        .iter()
        .find(|case| case.protocol == TransferProtocol::V2)
        .expect("pure-v2 case exists");
    let damaged_path = payload_root(&v2_case.save_path, &v2_case.fixture).join("alpha.bin");
    let mut damaged = std::fs::read(&damaged_path).expect("read pure-v2 payload");
    assert!(!damaged.is_empty());
    let damaged_file_length = damaged.len() as u64;
    damaged[0] ^= 0xff;
    std::fs::write(&damaged_path, damaged).expect("damage pure-v2 payload between runs");

    let restored_registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(receive_config), Arc::clone(&restored_registry))
        .await
        .expect("restart engine from persisted metainfo and database");
    // Pure-v2 actors re-hash their files during startup, including when the
    // user left the torrent paused. Wait for that verification result before
    // inspecting the restored projection; the actor starts asynchronously.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let registry = restored_registry.read().await;
            let startup_recheck_finished = cases.iter().all(|case| {
                registry.get(&case.info_hash).is_some_and(|entry| {
                    let expected_left = if case.protocol == TransferProtocol::V2 {
                        damaged_file_length
                    } else {
                        0
                    };
                    entry.state == TorrentState::Paused && entry.amount_left == expected_left
                })
            });
            if startup_recheck_finished {
                break;
            }
            drop(registry);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup verification must publish the restored progress projection");
    {
        let registry = restored_registry.read().await;
        assert_eq!(
            registry.len(),
            cases.len(),
            "all protocol identities restore"
        );
        for case in &cases {
            let restored = registry
                .get(&case.info_hash)
                .unwrap_or_else(|| panic!("missing restored {:?} torrent", case.protocol));
            assert_eq!(restored.state, TorrentState::Paused, "{:?}", case.protocol);
            let expected_left = if case.protocol == TransferProtocol::V2 {
                damaged_file_length
            } else {
                0
            };
            assert_eq!(
                restored.amount_left, expected_left,
                "{:?} startup verification progress",
                case.protocol,
            );
        }
    }

    for case in &cases {
        engine
            .resume_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "cannot resume restored {:?} fixture: {error}",
                    case.protocol
                )
            });
        let expected = if case.protocol == TransferProtocol::V2 {
            TorrentState::Downloading
        } else {
            TorrentState::Seeding
        };
        let amount_left = wait_for_state(
            &engine,
            &restored_registry,
            &case.info_hash,
            expected,
            &format!("restored {:?}", case.protocol),
        )
        .await;
        if case.protocol == TransferProtocol::V2 {
            assert!(amount_left > 0, "damaged pure-v2 payload is incomplete");
        } else {
            assert_eq!(
                amount_left, 0,
                "valid {:?} payload remains complete",
                case.protocol
            );
        }
    }

    engine.shutdown().await;
}

#[tokio::test]
async fn failed_live_transfer_cutover_restores_the_previous_owner() {
    let temp = tempfile::tempdir().unwrap();
    let config = test_config(temp.path(), unused_tcp_port());
    std::fs::create_dir_all(&config.storage.download_dir).unwrap();
    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
        .await
        .expect("engine start");
    let fixture = rt_testkit::public_transfer_compatibility_fixture(TransferProtocol::V1);
    let save_path = config.storage.download_dir.join("cutover-rollback");
    let info_hash = engine
        .add_torrent_raw_with_labels(fixture.torrent, Some(save_path), false, None, Vec::new())
        .await
        .expect("add active rollback fixture");

    // Fail the durable phase transition after the old actor has drained and
    // stopped. The rollback path must restart the old owner before reporting
    // the failed cutover to the caller.
    {
        let connection = rusqlite::Connection::open(config.db_path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_transfer_cutover_start
                 BEFORE UPDATE OF phase ON transfer_cutovers
                 WHEN NEW.phase = 'starting'
                 BEGIN SELECT RAISE(ABORT, 'injected transfer cutover failure'); END;",
            )
            .unwrap();
    }
    let error = engine
        .set_transfer_owner(info_hash.clone(), rt_db::TransferOwner::Reference)
        .await
        .expect_err("injected durable phase failure must fail the route change");
    assert!(
        error.contains("old-owner drain"),
        "unexpected error: {error}"
    );

    let connection = rusqlite::Connection::open(config.db_path()).unwrap();
    let route = rt_db::get_transfer_cutover(&connection, &info_hash)
        .unwrap()
        .expect("route journal remains present");
    assert_eq!(route.owner, rt_db::TransferOwner::SharedCore);
    assert_eq!(route.target, rt_db::TransferOwner::SharedCore);
    assert_eq!(route.phase, rt_db::TransferCutoverPhase::Stable);
    assert!(
        registry
            .read()
            .await
            .get(&info_hash)
            .is_some_and(|entry| entry.state != TorrentState::Error),
        "previous actor route should be live after rollback"
    );
    drop(connection);
    engine.shutdown().await;
}
