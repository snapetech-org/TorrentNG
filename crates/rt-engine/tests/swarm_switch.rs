use rt_config::Config;
use rt_engine::engine::swarm_switch::SwarmVariant;
use rt_engine::Engine;
use rt_session::{SessionRegistry, TorrentState};
use sha1::{Digest, Sha1};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::RwLock,
    time::timeout,
};

fn public_hash(content: &[u8]) -> String {
    match rt_metainfo::parse_torrent(&raw(false, content)).unwrap() {
        rt_metainfo::TorrentMeta::V1(meta) => hex::encode(meta.info_hash),
        _ => unreachable!(),
    }
}
use rt_bencode::{encode, BValue};

fn raw(private: bool, content: &[u8]) -> Vec<u8> {
    let pieces: Vec<u8> = content
        .chunks(16384)
        .flat_map(|c| Sha1::digest(c).to_vec())
        .collect();
    let mut fields = vec![
        (b"length".as_slice(), BValue::Int(content.len() as i64)),
        (b"name", BValue::Bytes(b"payload.bin")),
        (b"piece length", BValue::Int(16384)),
        (b"pieces", BValue::Bytes(&pieces)),
    ];
    if private {
        fields.push((b"private", BValue::Int(1)));
    }
    encode(&BValue::Dict(vec![(b"info", BValue::Dict(fields))]))
}

fn config(path: &Path) -> Config {
    let mut config = Config::default();
    config.daemon.session_dir = path.join("session");
    config.storage.download_dir = path.join("data");
    config.db.path = path.join("state.db");
    config.network.listen_port = 0;
    config.dht.port = 0;
    config.dht.enabled = true;
    config.dht.bootstrap_nodes.clear();
    config.tracker.allow_loopback_egress = true;
    config.daemon.shutdown_timeout_secs = 2;
    config
}

async fn wait_state(reg: &Arc<RwLock<SessionRegistry>>, hash: &str, desired: TorrentState) {
    timeout(Duration::from_secs(20), async {
        loop {
            if reg
                .read()
                .await
                .get(hash)
                .is_some_and(|t| t.state == desired)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("torrent did not reach {desired}"));
}

#[tokio::test]
async fn swarm_public_download_restart_and_private_restore_share_verified_payload() {
    let target_dir = tempfile::tempdir().unwrap();
    let target_config = config(target_dir.path());
    let content = (0..65539).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    let public = public_hash(&content);
    // A protocol fixture has its own peer ID; two Engine instances in one
    // process intentionally share the product peer ID and reject self-peering.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let seed_address = listener.local_addr().unwrap();
    let payload = content.clone();
    let expected_hash = public.clone();
    let seed = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 68];
        stream.read_exact(&mut bytes).await.unwrap();
        let incoming = rt_peer_wire::Handshake::parse(&bytes).unwrap();
        assert_eq!(hex::encode(incoming.info_hash), expected_hash);
        stream
            .write_all(&rt_peer_wire::Handshake::new(incoming.info_hash, [77; 20]).encode())
            .await
            .unwrap();
        stream
            .write_all(&[0, 0, 0, 2, 5, 0xf8, 0, 0, 0, 1, 1])
            .await
            .unwrap(); // 5 pieces, unchoke
        let mut sent = 0;
        while let Ok(length) = stream.read_u32().await {
            if length == 0 {
                continue;
            }
            assert!(length < 64 * 1024);
            let mut message = vec![0; length as usize];
            stream.read_exact(&mut message).await.unwrap();
            if message[0] == 6 {
                assert_eq!(length, 13);
                let piece = u32::from_be_bytes(message[1..5].try_into().unwrap()) as usize;
                let offset = u32::from_be_bytes(message[5..9].try_into().unwrap()) as usize;
                let len = u32::from_be_bytes(message[9..13].try_into().unwrap()) as usize;
                let start = piece * 16384 + offset;
                stream.write_u32((len + 9) as u32).await.unwrap();
                stream.write_u8(7).await.unwrap();
                stream.write_all(&message[1..9]).await.unwrap();
                stream
                    .write_all(&payload[start..start + len])
                    .await
                    .unwrap();
                sent += len;
            }
        }
        assert_eq!(sent, payload.len());
    });

    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let target = Engine::start(Arc::new(target_config.clone()), Arc::clone(&registry))
        .await
        .unwrap();
    let private_raw = raw(true, &content);
    let original = target
        .add_torrent_raw_with_labels(
            private_raw.clone(),
            None,
            true,
            Some("books".into()),
            vec!["keep".into()],
        )
        .await
        .unwrap();
    assert_eq!(public_hash(&content), public);
    let switched = target
        .switch_to_public(
            original.clone(),
            SwarmVariant::RemovePrivate,
            None,
            public.clone(),
        )
        .await
        .unwrap();
    assert!(switched.can_restore);
    assert_eq!(
        registry.read().await.get(&original).unwrap().state,
        TorrentState::Paused
    );
    assert!(target
        .resume_torrent(original.clone())
        .await
        .unwrap_err()
        .contains("parked"));
    assert!(target.recheck_torrent(original.clone()).await.is_err());
    assert!(target.remove_torrent(original.clone(), true).await.is_err());
    assert!(target.remove_torrent(public.clone(), false).await.is_err());
    assert!(target
        .update_torrent_fields(public.clone(), None, Some(target_dir.path().join("moved")))
        .await
        .is_err());
    assert!(target
        .update_torrent_trackers(
            public.clone(),
            vec!["https://private.invalid/passkey/announce".into()]
        )
        .await
        .is_err());
    assert_eq!(
        target.torrent_blob(original.clone()).await.unwrap(),
        private_raw
    );
    assert!(
        !target
            .torrent_metadata(public.clone())
            .await
            .unwrap()
            .is_private
    );
    wait_state(&registry, &public, TorrentState::Downloading).await;
    target
        .add_peers(public.clone(), vec![seed_address])
        .await
        .unwrap();
    wait_state(&registry, &public, TorrentState::Seeding).await;
    assert_eq!(
        std::fs::read(target_config.storage.download_dir.join("payload.bin")).unwrap(),
        content
    );
    let public_entry = registry.read().await.get(&public).unwrap();
    assert_eq!(public_entry.category.as_deref(), Some("books"));
    assert_eq!(public_entry.tags, vec!["keep"]);
    target.shutdown().await;

    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let target = Engine::start(Arc::new(target_config.clone()), Arc::clone(&registry))
        .await
        .unwrap();
    assert!(
        target
            .swarm_status(original.clone())
            .await
            .unwrap()
            .can_restore
    );
    assert_eq!(
        registry.read().await.get(&original).unwrap().state,
        TorrentState::Paused
    );
    target.restore_private_swarm(public.clone()).await.unwrap();
    wait_state(&registry, &original, TorrentState::Seeding).await;
    assert!(registry.read().await.get(&public).is_none());
    assert_eq!(
        target.torrent_blob(original.clone()).await.unwrap(),
        private_raw
    );
    assert!(
        target
            .torrent_metadata(original.clone())
            .await
            .unwrap()
            .is_private
    );
    assert!(
        !target
            .swarm_status(original.clone())
            .await
            .unwrap()
            .can_restore
    );
    target.restore_private_swarm(original).await.unwrap(); // retry is idempotent
    target.shutdown().await;
    seed.await.unwrap();
}

#[tokio::test]
async fn swarm_deleting_private_copy_leaves_independent_public_and_files() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
        .await
        .unwrap();
    let content = b"retain this payload";
    std::fs::write(config.storage.download_dir.join("payload.bin"), content).unwrap();
    let original = engine
        .add_torrent_raw_with_labels(raw(true, content), None, true, None, vec![])
        .await
        .unwrap();
    let public = public_hash(content);
    engine
        .switch_to_public(
            original.clone(),
            SwarmVariant::RemovePrivate,
            None,
            public.clone(),
        )
        .await
        .unwrap();
    engine
        .remove_torrent(original.clone(), false)
        .await
        .unwrap();
    assert!(registry.read().await.get(&original).is_none());
    let status = engine.swarm_status(public.clone()).await.unwrap();
    assert_eq!(status.phase, "public_independent");
    assert!(!status.can_restore);
    assert!(!status.can_switch);
    assert!(engine.restore_private_swarm(public.clone()).await.is_err());
    assert_eq!(
        std::fs::read(config.storage.download_dir.join("payload.bin")).unwrap(),
        content
    );
    assert!(engine.torrent_blob(original.clone()).await.is_err());
    engine.shutdown().await;
    let engine = Engine::start(
        Arc::new(config),
        Arc::new(RwLock::new(SessionRegistry::new())),
    )
    .await
    .unwrap();
    assert!(!engine.swarm_status(public).await.unwrap().can_restore);
    engine.shutdown().await;
}

#[tokio::test]
async fn swarm_generated_hash_requires_an_explicit_matching_preview() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(config), registry).await.unwrap();
    let original = engine
        .add_torrent_raw_with_labels(raw(true, b"abc"), None, true, None, vec![])
        .await
        .unwrap();
    assert!(engine
        .switch_to_public(
            original.clone(),
            SwarmVariant::RemovePrivate,
            None,
            "a".repeat(40)
        )
        .await
        .unwrap_err()
        .contains("Candidate changed"));
    assert!(engine.swarm_status(original).await.unwrap().can_switch);
    let public = engine
        .add_torrent_raw_with_labels(raw(false, b"xyz"), None, true, None, vec![])
        .await
        .unwrap();
    assert!(
        !engine
            .swarm_status(public.clone())
            .await
            .unwrap()
            .can_restore
    );
    assert!(engine.restore_private_swarm(public).await.is_err());
    engine.shutdown().await;
}

#[tokio::test]
async fn swarm_interrupted_phases_park_both_and_restore_even_before_public_creation() {
    for (phase, create_public) in [
        ("preparing", false),
        ("preparing", true),
        ("restoring", true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let config = config(temp.path());
        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        let engine = Engine::start(Arc::new(config.clone()), registry)
            .await
            .unwrap();
        let content = b"crash-recovery-fixture";
        std::fs::write(config.storage.download_dir.join("payload.bin"), content).unwrap();
        let original = engine
            .add_torrent_raw_with_labels(raw(true, content), None, true, None, vec![])
            .await
            .unwrap();
        let public = public_hash(content);
        if create_public {
            engine
                .switch_to_public(
                    original.clone(),
                    SwarmVariant::RemovePrivate,
                    None,
                    public.clone(),
                )
                .await
                .unwrap();
        }
        engine.shutdown().await;
        {
            let mut db = rusqlite::Connection::open(&config.db.path).unwrap();
            if !create_public {
                rt_db::swarm_switch_row::begin(
                    &mut db,
                    &rt_db::swarm_switch_row::SwarmSwitchRow {
                        original_hash: original.clone(),
                        public_hash: public.clone(),
                        phase: phase.into(),
                        updated_at: 0,
                    },
                )
                .unwrap();
            }
            rt_db::swarm_switch_row::set_phase(&db, &original, phase, 0).unwrap();
        }
        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        let engine = Engine::start(Arc::new(config), Arc::clone(&registry))
            .await
            .unwrap();
        assert_eq!(
            engine.swarm_status(original.clone()).await.unwrap().phase,
            phase
        );
        assert_eq!(
            registry.read().await.get(&original).unwrap().state,
            TorrentState::Paused
        );
        assert!(engine.resume_torrent(original.clone()).await.is_err());
        if create_public {
            assert_eq!(
                registry.read().await.get(&public).unwrap().state,
                TorrentState::Paused
            );
            assert!(engine.resume_torrent(public.clone()).await.is_err());
        }
        engine
            .restore_private_swarm(original.clone())
            .await
            .unwrap();
        wait_state(&registry, &original, TorrentState::Seeding).await;
        assert!(registry.read().await.get(&public).is_none());
        engine.shutdown().await;
    }
}

#[tokio::test]
async fn swarm_missing_private_artifact_never_retires_public_or_invents_a_restore() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let registry = Arc::new(RwLock::new(SessionRegistry::new()));
    let engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
        .await
        .unwrap();
    let content = b"keep seeding this file";
    std::fs::write(config.storage.download_dir.join("payload.bin"), content).unwrap();
    let original = engine
        .add_torrent_raw_with_labels(raw(true, content), None, true, None, vec![])
        .await
        .unwrap();
    let public = public_hash(content);
    engine
        .switch_to_public(
            original.clone(),
            SwarmVariant::RemovePrivate,
            None,
            public.clone(),
        )
        .await
        .unwrap();
    std::fs::remove_file(
        config
            .daemon
            .session_dir
            .join("torrents")
            .join(format!("{original}.torrent")),
    )
    .unwrap();
    assert!(engine
        .restore_private_swarm(original.clone())
        .await
        .unwrap_err()
        .contains("unavailable"));
    assert!(registry.read().await.get(&public).is_some());
    assert_eq!(
        engine.swarm_status(public.clone()).await.unwrap().phase,
        "public"
    );
    engine.remove_torrent(original, false).await.unwrap();
    assert!(!engine.swarm_status(public).await.unwrap().can_restore);
    assert_eq!(
        std::fs::read(config.storage.download_dir.join("payload.bin")).unwrap(),
        content
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn swarm_v2_and_hybrid_switch_and_restore_verify_the_same_file() {
    for hybrid in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let config = config(temp.path());
        let content = b"authenticated v2 content";
        let root = rt_hash::BlockHash::of(content).0;
        let piece = Sha1::digest(content);
        let mut info = vec![
            (
                b"file tree".as_slice(),
                BValue::Dict(vec![(
                    b"payload.bin",
                    BValue::Dict(vec![(
                        b"",
                        BValue::Dict(vec![
                            (b"length", BValue::Int(content.len() as i64)),
                            (b"pieces root", BValue::Bytes(&root)),
                        ]),
                    )]),
                )]),
            ),
            (b"meta version", BValue::Int(2)),
            (b"name", BValue::Bytes(b"payload.bin")),
            (b"piece length", BValue::Int(16384)),
            (b"private", BValue::Int(1)),
        ];
        if hybrid {
            info.push((b"length", BValue::Int(content.len() as i64)));
            info.push((b"pieces", BValue::Bytes(&piece)));
        }
        info.sort_by(|a, b| a.0.cmp(b.0));
        let raw = encode(&BValue::Dict(vec![(b"info", BValue::Dict(info))]));
        let candidate = rt_metainfo::swarm::public_counterpart(
            &raw,
            Some(rt_metainfo::swarm::PublicVariant::RemovePrivate),
        )
        .unwrap();
        let public = match candidate {
            rt_metainfo::TorrentMeta::V2(m) => hex::encode(m.info_hash_v2),
            rt_metainfo::TorrentMeta::Hybrid(m, _) => hex::encode(m.info_hash),
            _ => unreachable!(),
        };
        let registry = Arc::new(RwLock::new(SessionRegistry::new()));
        let engine = Engine::start(Arc::new(config.clone()), Arc::clone(&registry))
            .await
            .unwrap();
        std::fs::write(config.storage.download_dir.join("payload.bin"), content).unwrap();
        let original = engine
            .add_torrent_raw_with_labels(raw.clone(), None, true, None, vec![])
            .await
            .unwrap();
        engine
            .switch_to_public(
                original.clone(),
                SwarmVariant::RemovePrivate,
                None,
                public.clone(),
            )
            .await
            .unwrap();
        wait_state(&registry, &public, TorrentState::Seeding).await;
        engine
            .restore_private_swarm(original.clone())
            .await
            .unwrap();
        wait_state(&registry, &original, TorrentState::Seeding).await;
        assert_eq!(engine.torrent_blob(original).await.unwrap(), raw);
        assert!(registry.read().await.get(&public).is_none());
        engine.shutdown().await;
    }
}
