//! End-to-end: launch a real `torrentngd` the way the app does (Local mode),
//! then drive the same client + SSE path the UI uses.
//!
//! Needs a built daemon, so it is opt-in: set `TORRENTNGD_BIN=/path/to/torrentngd`.
//! CI builds the daemon and sets it; locally it is skipped when unset.

use std::time::Duration;

use futures_util::StreamExt;
use torrentng_desktop::api::{AddSource, ApiError, Client, SseParser};
use torrentng_desktop::backend;
use torrentng_desktop::config::LocalConfig;
use torrentng_desktop::model::{Delta, ViewSpec};

const HASH: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

async fn eventually(
    client: &Client,
    spec: &ViewSpec,
    total: usize,
) -> torrentng_desktop::model::ListResponse {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let r = client.list(spec, 0, 200, None).await.unwrap();
        if r.total == total {
            return r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "list never reached total={total} (last {})",
            r.total
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_engine_lifecycle_and_delta_stream() {
    let Ok(bin) = std::env::var("TORRENTNGD_BIN") else {
        eprintln!("skipped: TORRENTNGD_BIN not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let local = LocalConfig {
        binary: bin,
        data_dir: dir.path().join("data").display().to_string(),
        download_dir: dir.path().join("dl").display().to_string(),
        listen_port: backend::free_port().unwrap(),
        ..LocalConfig::default()
    };

    let engine = backend::start(&local)
        .await
        .expect("engine starts and becomes healthy");
    assert!(engine.url.starts_with("http://127.0.0.1:"), "loopback only");

    // A wrong token is rejected, the right one is accepted.
    let bad = Client::new(&engine.url, "wrong").unwrap();
    assert_eq!(bad.verify().await, Err(ApiError::Unauthorized));
    let client = Client::new(&engine.url, &engine.token).unwrap();
    client.verify().await.expect("token accepted");

    // Empty library; remember the revision the SSE stream will resume from.
    let spec = ViewSpec::default();
    let first = client.list(&spec, 0, 200, None).await.unwrap();
    assert_eq!(first.total, 0);

    // Open the delta stream *before* the change so we observe it live.
    let resp = client.events(first.snapshot).await.expect("sse opens");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Delta>();
    let reader = tokio::spawn(async move {
        let mut parser = SseParser::default();
        let mut stream = resp.bytes_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            for ev in parser.feed(&chunk) {
                if ev.event == "torrent_delta" {
                    if let Ok(d) = serde_json::from_str::<Delta>(&ev.data) {
                        let _ = tx.send(d);
                    }
                }
            }
        }
    });

    let magnet = format!("magnet:?xt=urn:btih:{HASH}&dn=desktop-roundtrip");
    client
        .add(AddSource::Magnet(magnet), "", false)
        .await
        .expect("magnet accepted");

    let delta = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let d = rx.recv().await.expect("stream stays open");
            if d.torrents.iter().any(|t| t.info_hash == HASH) {
                return d;
            }
        }
    })
    .await
    .expect("delta for the new torrent arrives over SSE");
    assert!(
        !delta.snapshot,
        "resuming from a revision must not resend the library"
    );

    // The daemon serves list snapshots up to ~750 ms old; wait for it.
    let after = eventually(&client, &spec, 1).await;
    assert_eq!(after.torrents[0].info_hash, HASH);

    let detail = client.detail(HASH).await.unwrap();
    assert_eq!(detail.summary.info_hash, HASH);

    let live = client.live(&[HASH.to_owned()]).await.unwrap();
    assert_eq!(live.len(), 1);
    client.transfer_info().await.unwrap();
    client.facets().await.unwrap();

    client.action(HASH, "stop").await.unwrap();
    client.action(HASH, "start").await.unwrap();

    // Server-side filtering narrows the page.
    let none = ViewSpec {
        filter: "no-such-name".into(),
        ..ViewSpec::default()
    };
    assert_eq!(client.list(&none, 0, 200, None).await.unwrap().total, 0);

    client.remove(HASH, false).await.unwrap();
    eventually(&client, &spec, 0).await;

    reader.abort();
    // Graceful stop must return promptly (SIGTERM path), not hit the 15s kill.
    let t = std::time::Instant::now();
    engine.stop().await;
    assert!(
        t.elapsed() < Duration::from_secs(14),
        "engine shut down gracefully"
    );
}
