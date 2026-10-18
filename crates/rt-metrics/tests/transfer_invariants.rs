//! Full-engine protocol and restart invariants for production-shaped fixtures.
//!
//! These cases exercise metainfo admission, disk recheck, persisted completion
//! state, clean restart, and the recheck performed when a dormant torrent is
//! resumed. They are correctness gates, not transfer-rate measurements.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rt_config::Config;
use rt_engine::Engine;
use rt_session::{SessionRegistry, TorrentState};
use rt_testkit::{transfer_compatibility_matrix, TransferCompatibilityFixture, TransferProtocol};
use tokio::sync::RwLock;

const TEST_TRACKER: &str = "http://127.0.0.1:1/announce";

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

fn materialize_fixture(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let torrent_root = save_path.join("transfer-compatibility");
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
    let torrent_root = save_path.join("transfer-compatibility");
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
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .expect("reserve a loopback port")
        .port()
}

fn assert_fixture_contents(
    save_path: &Path,
    fixture: &TransferCompatibilityFixture,
) -> std::io::Result<()> {
    let torrent_root = save_path.join("transfer-compatibility");
    for file in &fixture.payload_files {
        let mut path = torrent_root.clone();
        for component in &file.path {
            path.push(*component);
        }
        assert_eq!(
            std::fs::read(path)?,
            file.bytes,
            "payload file {:?}",
            file.path
        );
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

async fn wait_for_state(
    registry: &Arc<RwLock<SessionRegistry>>,
    info_hash: &str,
    expected: TorrentState,
) -> u64 {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let observed = {
                let registry = registry.read().await;
                registry
                    .get(info_hash)
                    .map(|entry| (entry.state, entry.amount_left))
            };
            match observed {
                Some((state, amount_left)) if state == expected => return amount_left,
                Some((TorrentState::Error, _)) => {
                    panic!("torrent {info_hash} entered Error while waiting for {expected:?}")
                }
                Some(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                None => panic!("torrent {info_hash} disappeared while waiting for {expected:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("torrent {info_hash} did not reach {expected:?}"))
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
    config
}

#[tokio::test]
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

    for fixture in transfer_compatibility_matrix() {
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
            .update_torrent_trackers(info_hash.clone(), vec![TEST_TRACKER.to_owned()])
            .await
            .unwrap_or_else(|error| panic!("cannot pin seed tracker for {info_hash}: {error}"));
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
            .update_torrent_trackers(receive_hash.clone(), vec![TEST_TRACKER.to_owned()])
            .await
            .unwrap_or_else(|error| {
                panic!("cannot pin receiver tracker for {receive_hash}: {error}")
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
        let amount_left =
            wait_for_state(&seed_registry, &case.info_hash, TorrentState::Seeding).await;
        assert_eq!(amount_left, 0, "{:?} seed is complete", case.protocol);
    }
    let seed_peer = SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        seed_engine.listen_port().await.unwrap(),
    ));

    for case in &cases {
        receive_engine
            .resume_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot start {:?} receiver: {error}", case.protocol));
        wait_for_state(
            &receive_registry,
            &case.info_hash,
            TorrentState::Downloading,
        )
        .await;
        receive_engine
            .add_peers(case.info_hash.clone(), vec![seed_peer])
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "cannot connect {:?} receiver to seed: {error}",
                    case.protocol
                )
            });
        let amount_left =
            wait_for_state(&receive_registry, &case.info_hash, TorrentState::Seeding).await;
        assert_eq!(amount_left, 0, "{:?} transfer is complete", case.protocol);
        assert_fixture_contents(&case.save_path, &case.fixture).unwrap_or_else(|error| {
            panic!("{:?} downloaded payload is invalid: {error}", case.protocol)
        });
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
    }
    seed_engine.shutdown().await;
    receive_engine.shutdown().await;

    let v2_case = cases
        .iter()
        .find(|case| case.protocol == TransferProtocol::V2)
        .expect("pure-v2 case exists");
    let damaged_path = v2_case
        .save_path
        .join("transfer-compatibility")
        .join("alpha.bin");
    let mut damaged = std::fs::read(&damaged_path).expect("read pure-v2 payload");
    assert!(!damaged.is_empty());
    damaged[0] ^= 0xff;
    std::fs::write(&damaged_path, damaged).expect("damage pure-v2 payload between runs");

    let restored_registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(receive_config), Arc::clone(&restored_registry))
        .await
        .expect("restart engine from persisted metainfo and database");
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
            assert_eq!(restored.amount_left, 0, "persisted complete state");
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
        let amount_left = wait_for_state(&restored_registry, &case.info_hash, expected).await;
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
