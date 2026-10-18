//! Full-engine protocol and restart invariants for production-shaped fixtures.
//!
//! These cases exercise metainfo admission, disk recheck, persisted completion
//! state, clean restart, and the recheck performed when a dormant torrent is
//! resumed. They are correctness gates, not transfer-rate measurements.

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

fn test_config(root: &Path) -> Config {
    let mut config = Config::default();
    config.daemon.session_dir = root.join("session");
    config.daemon.shutdown_timeout_secs = 30;
    config.db.path = root.join("state.sqlite");
    config.storage.download_dir = root.join("downloads");
    config.network.listen_port = 0;
    config.dht.enabled = false;
    config.runtime.torrent_tiers_enabled = false;
    config
}

#[tokio::test]
async fn persisted_v2_completion_is_rechecked_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let config = test_config(temp.path());
    let download_dir = config.storage.download_dir.clone();
    std::fs::create_dir_all(&download_dir).unwrap();

    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
        .await
        .expect("first engine start");
    let mut cases = Vec::new();

    for fixture in transfer_compatibility_matrix() {
        let save_path = download_dir.join(protocol_label(fixture.protocol));
        materialize_fixture(&save_path, &fixture).expect("materialize protocol payload");
        let info_hash = engine
            .add_torrent_raw_with_labels(
                fixture.torrent.clone(),
                Some(save_path.clone()),
                true,
                None,
                Vec::new(),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("engine rejected {:?} metainfo: {error}", fixture.protocol)
            });
        engine
            .update_torrent_trackers(info_hash.clone(), vec![TEST_TRACKER.to_owned()])
            .await
            .unwrap_or_else(|error| panic!("cannot pin local tracker for {info_hash}: {error}"));
        cases.push(ProtocolCase {
            protocol: fixture.protocol,
            info_hash,
            save_path,
            fixture,
        });
    }

    for case in &cases {
        engine
            .resume_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot start {:?} fixture: {error}", case.protocol));
        let amount_left = wait_for_state(&registry, &case.info_hash, TorrentState::Seeding).await;
        assert_eq!(amount_left, 0, "{:?} fixture is complete", case.protocol);
    }
    for case in &cases {
        engine
            .pause_torrent(case.info_hash.clone())
            .await
            .unwrap_or_else(|error| panic!("cannot pause {:?} fixture: {error}", case.protocol));
    }
    engine.shutdown().await;

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
    let engine = Engine::start(Arc::new(config), Arc::clone(&restored_registry))
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
