#![deny(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("the pinned urtorrent comparator is Linux-only");

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use urtorrent::{AddTorrent, Session, TorrentId, TorrentState};

const SETTINGS_FORMAT: &str = "transfer-urtorrent-settings-v1";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Download,
    Seed,
}

struct Args {
    mode: Mode,
    namespace: String,
    torrent: PathBuf,
    save_path: PathBuf,
    pid_file: PathBuf,
    settings_file: PathBuf,
    start_gate: PathBuf,
    ready_file: PathBuf,
    stop_file: PathBuf,
    listen_v4: Ipv4Addr,
    peer: SocketAddr,
    max_connections: usize,
    max_peers: usize,
    unchoke_slots: usize,
    hash_threads: usize,
    upload_limit: u64,
    download_limit: u64,
    timeout_seconds: u64,
    seed_seconds: u64,
}

fn usage() -> &'static str {
    "urtorrent-runner --mode download|seed --namespace tngt-NAME --torrent FILE \
     --save-path DIR --pid-file FILE --settings-file FILE --start-gate FILE --ready-file FILE \
     --stop-file FILE \
     --listen-v4 IP --peer IPv4:PORT [--max-connections N] [--max-peers N] \
     [--unchoke-slots N] [--hash-threads N] [--upload-limit BPS] \
     [--download-limit BPS] [--timeout-seconds N] [--seed-seconds N]\n\
     urtorrent-runner preflight --torrent FILE --save-path EMPTY_DIR"
}

fn parse_preflight_args(
    values: impl IntoIterator<Item = String>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let mut values = values.into_iter();
    let mut seen = HashSet::new();
    let mut torrent = None;
    let mut save_path = None;
    while let Some(name) = values.next() {
        if !seen.insert(name.clone()) {
            bail!("preflight argument {name} may only be supplied once");
        }
        let value = values
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing value for {name}"))?;
        match name.as_str() {
            "--torrent" => torrent = Some(PathBuf::from(value)),
            "--save-path" => save_path = Some(PathBuf::from(value)),
            _ => bail!("unknown preflight argument {name}"),
        }
    }
    let torrent = torrent.ok_or_else(|| anyhow::anyhow!("--torrent is required"))?;
    let save_path = save_path.ok_or_else(|| anyhow::anyhow!("--save-path is required"))?;
    if !torrent.is_absolute() || !save_path.is_absolute() {
        bail!("preflight torrent and save paths must be absolute");
    }
    if torrent == save_path {
        bail!("preflight torrent and save paths must be distinct");
    }
    Ok((torrent, save_path))
}

fn parse_value<T: std::str::FromStr>(name: &str, value: &str) -> anyhow::Result<T>
where
    T::Err: std::fmt::Display + Send + Sync + 'static,
{
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid {name}: {error}"))
}

fn is_task_local_ip(address: Ipv4Addr) -> bool {
    address.is_loopback() || address.is_private() || address.is_link_local()
}

fn is_help_flag(value: &str) -> bool {
    matches!(value, "--help" | "-h")
}

fn parse_trial_args(values: impl IntoIterator<Item = String>) -> anyhow::Result<Args> {
    let mut values = values.into_iter();
    let mut seen = HashSet::new();
    let mut mode = None;
    let mut namespace = None;
    let mut torrent = None;
    let mut save_path = None;
    let mut pid_file = None;
    let mut settings_file = None;
    let mut start_gate = None;
    let mut ready_file = None;
    let mut stop_file = None;
    let mut listen_v4 = None;
    let mut peer = None;
    let mut max_connections = 500usize;
    let mut max_peers = 50usize;
    let mut unchoke_slots = 8usize;
    let mut hash_threads = 2usize;
    let mut upload_limit = 0u64;
    let mut download_limit = 0u64;
    let mut timeout_seconds = 1800u64;
    let mut seed_seconds = 60u64;

    while let Some(name) = values.next() {
        if is_help_flag(&name) {
            println!("{}", usage());
            std::process::exit(0);
        }
        if !seen.insert(name.clone()) {
            bail!("argument {name} may only be supplied once");
        }
        let value = values
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing value for {name}"))?;
        match name.as_str() {
            "--mode" => {
                mode = Some(match value.as_str() {
                    "download" => Mode::Download,
                    "seed" => Mode::Seed,
                    _ => bail!("--mode must be download or seed"),
                });
            }
            "--namespace" => namespace = Some(value),
            "--torrent" => torrent = Some(PathBuf::from(value)),
            "--save-path" => save_path = Some(PathBuf::from(value)),
            "--pid-file" => pid_file = Some(PathBuf::from(value)),
            "--settings-file" => settings_file = Some(PathBuf::from(value)),
            "--start-gate" => start_gate = Some(PathBuf::from(value)),
            "--ready-file" => ready_file = Some(PathBuf::from(value)),
            "--stop-file" => stop_file = Some(PathBuf::from(value)),
            "--listen-v4" => listen_v4 = Some(parse_value("IPv4 listen address", &value)?),
            "--peer" => peer = Some(parse_value("peer address", &value)?),
            "--max-connections" => max_connections = parse_value(&name, &value)?,
            "--max-peers" => max_peers = parse_value(&name, &value)?,
            "--unchoke-slots" => unchoke_slots = parse_value(&name, &value)?,
            "--hash-threads" => hash_threads = parse_value(&name, &value)?,
            "--upload-limit" => upload_limit = parse_value(&name, &value)?,
            "--download-limit" => download_limit = parse_value(&name, &value)?,
            "--timeout-seconds" => timeout_seconds = parse_value(&name, &value)?,
            "--seed-seconds" => seed_seconds = parse_value(&name, &value)?,
            _ => bail!("unknown argument {name}; {}", usage()),
        }
    }

    let args = Args {
        mode: mode.ok_or_else(|| anyhow::anyhow!("--mode is required"))?,
        namespace: namespace.ok_or_else(|| anyhow::anyhow!("--namespace is required"))?,
        torrent: torrent.ok_or_else(|| anyhow::anyhow!("--torrent is required"))?,
        save_path: save_path.ok_or_else(|| anyhow::anyhow!("--save-path is required"))?,
        pid_file: pid_file.ok_or_else(|| anyhow::anyhow!("--pid-file is required"))?,
        settings_file: settings_file
            .ok_or_else(|| anyhow::anyhow!("--settings-file is required"))?,
        start_gate: start_gate.ok_or_else(|| anyhow::anyhow!("--start-gate is required"))?,
        ready_file: ready_file.ok_or_else(|| anyhow::anyhow!("--ready-file is required"))?,
        stop_file: stop_file.ok_or_else(|| anyhow::anyhow!("--stop-file is required"))?,
        listen_v4: listen_v4
            .ok_or_else(|| anyhow::anyhow!("--listen-v4 must bind a private task interface"))?,
        peer: peer.ok_or_else(|| {
            anyhow::anyhow!("--peer is required; this comparator uses a pinned local peer only")
        })?,
        max_connections,
        max_peers,
        unchoke_slots,
        hash_threads,
        upload_limit,
        download_limit,
        timeout_seconds,
        seed_seconds,
    };
    if args.peer.port() == 0 {
        bail!("--peer must use a nonzero port");
    }
    match args.peer {
        SocketAddr::V4(peer) if is_task_local_ip(*peer.ip()) => {}
        SocketAddr::V6(_) => bail!("--peer must use an IPv4 address; IPv6 is disabled"),
        _ => bail!("--peer must use a private, link-local, or loopback IPv4 address"),
    }
    if !is_task_local_ip(args.listen_v4) {
        bail!("--listen-v4 must use a private, link-local, or loopback IPv4 address");
    }
    for (name, path) in [
        ("--torrent", &args.torrent),
        ("--save-path", &args.save_path),
        ("--pid-file", &args.pid_file),
        ("--settings-file", &args.settings_file),
        ("--start-gate", &args.start_gate),
        ("--ready-file", &args.ready_file),
        ("--stop-file", &args.stop_file),
    ] {
        if !path.is_absolute() {
            bail!("{name} must be an absolute path");
        }
    }
    if args.pid_file == args.settings_file
        || args.pid_file == args.start_gate
        || args.settings_file == args.start_gate
        || args.pid_file == args.ready_file
        || args.settings_file == args.ready_file
        || args.start_gate == args.ready_file
        || args.pid_file == args.stop_file
        || args.settings_file == args.stop_file
        || args.start_gate == args.stop_file
        || args.ready_file == args.stop_file
    {
        bail!("runner control paths must be distinct");
    }
    let namespace_suffix = args.namespace.strip_prefix("tngt-").unwrap_or_default();
    if namespace_suffix.is_empty()
        || namespace_suffix.len() > 32
        || !(namespace_suffix.as_bytes()[0].is_ascii_lowercase()
            || namespace_suffix.as_bytes()[0].is_ascii_digit())
        || namespace_suffix.ends_with('-')
        || !namespace_suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("--namespace must be a task-owned tngt-* namespace");
    }
    if args.max_connections == 0
        || args.max_peers == 0
        || args.hash_threads == 0
        || args.timeout_seconds == 0
        || args.seed_seconds == 0
    {
        bail!("connection, worker, timeout, and seed durations must be positive");
    }
    if args.mode == Mode::Seed && args.seed_seconds > args.timeout_seconds {
        bail!("--seed-seconds cannot exceed --timeout-seconds");
    }
    Ok(args)
}

fn parse_args() -> anyhow::Result<Args> {
    parse_trial_args(std::env::args().skip(1))
}

fn assert_task_namespace(name: &str) -> anyhow::Result<()> {
    let path = Path::new("/run/netns").join(name);
    let link = fs::symlink_metadata(&path)
        .with_context(|| format!("cannot inspect task namespace {}", path.display()))?;
    if link.file_type().is_symlink() || !link.is_file() {
        bail!("task namespace handle must be a regular non-symlink file");
    }
    let expected = fs::metadata(&path).context("cannot stat task namespace handle")?;
    let current =
        fs::metadata("/proc/self/ns/net").context("cannot stat current network namespace")?;
    if expected.dev() != current.dev() || expected.ino() != current.ino() {
        bail!("runner is not executing inside the declared task network namespace");
    }
    Ok(())
}

fn process_start_ticks() -> anyhow::Result<u64> {
    let stat =
        fs::read_to_string("/proc/self/stat").context("cannot read runner process identity")?;
    let fields = stat
        .get(
            stat.rfind(')')
                .ok_or_else(|| anyhow::anyhow!("malformed process stat"))?
                + 2..,
        )
        .ok_or_else(|| anyhow::anyhow!("malformed process stat fields"))?;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow::anyhow!("process start time is missing"))?
        .parse()
        .context("process start time is malformed")
}

fn write_new_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create immutable file {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn session_settings(value: &urtorrent::SessionSettings) -> Value {
    json!({
        "listen_port": value.listen_port,
        "listen_v4": value.listen_v4.map(|address| address.to_string()),
        "listen_v6": value.listen_v6.map(|address| address.to_string()),
        "profile": value.profile,
        "dht": value.dht,
        "upload_limit": value.upload_limit,
        "download_limit": value.download_limit,
        "max_connections": value.max_connections,
        "max_peers_per_torrent": value.max_peers_per_torrent,
        "unchoke_slots": value.unchoke_slots,
        "active_limits": format!("{:?}", value.active_limits),
        "encryption": format!("{:?}", value.encryption),
        "transports": format!("{:?}", value.transports),
        "pex": value.pex,
        "lsd": value.lsd,
    })
}

fn write_identity_files(args: &Args, settings: Value) -> anyhow::Result<()> {
    let pid = std::process::id();
    let start_ticks = process_start_ticks()?;
    write_new_private(&args.pid_file, format!("{pid}\n").as_bytes())?;
    let snapshot = json!({
        "format": SETTINGS_FORMAT,
        "version": urtorrent::VERSION,
        "process_id": pid,
        "process_start_ticks": start_ticks,
        "settings": settings,
    });
    let mut bytes = serde_json::to_vec_pretty(&snapshot)?;
    bytes.push(b'\n');
    write_new_private(&args.settings_file, &bytes)
}

async fn wait_for_gate(path: &Path, deadline: Instant) -> anyhow::Result<()> {
    loop {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                return Ok(());
            }
            Ok(_) => bail!("start gate must be a regular non-symlink file"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect start gate"),
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for the benchmark start gate");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn run_trial() -> anyhow::Result<()> {
    let args = parse_args()?;
    assert_task_namespace(&args.namespace)?;
    match fs::symlink_metadata(&args.start_gate) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!("start gate must be absent until every trial participant is ready"),
        Err(error) => return Err(error).context("cannot inspect start gate before setup"),
    }
    match fs::symlink_metadata(&args.ready_file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!("ready marker must be absent before trial setup"),
        Err(error) => return Err(error).context("cannot inspect ready marker before setup"),
    }
    match fs::symlink_metadata(&args.stop_file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!("stop marker must be absent before trial setup"),
        Err(error) => return Err(error).context("cannot inspect stop marker before setup"),
    }
    let metadata = fs::symlink_metadata(&args.torrent)
        .with_context(|| format!("cannot inspect torrent file {}", args.torrent.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("torrent input must be a regular non-symlink file");
    }
    let torrent_bytes = fs::read(&args.torrent).context("cannot read torrent metainfo")?;
    let metainfo = urtorrent::Torrent::parse(&torrent_bytes)
        .context("cannot parse benchmark torrent metainfo")?;
    if metainfo.info.has_v2 {
        bail!("urtorrent comparator accepts v1-only metainfo, not v2 or hybrid fixtures");
    }
    if !metainfo.info.private {
        bail!("benchmark torrent metainfo must carry the private flag");
    }
    let save_metadata = fs::symlink_metadata(&args.save_path).with_context(|| {
        format!(
            "cannot inspect task-owned save directory {}",
            args.save_path.display()
        )
    })?;
    if save_metadata.file_type().is_symlink() || !save_metadata.is_dir() {
        bail!("save path must be an existing non-symlink directory owned by this trial");
    }
    if args.mode == Mode::Download
        && fs::read_dir(&args.save_path)
            .context("cannot inspect task-owned download directory")?
            .next()
            .is_some()
    {
        bail!("download directory must be empty before the trial starts");
    }

    let session = Session::builder()
        .listen_port(0)
        .listen_v4(Some(args.listen_v4))
        .listen_v6(None)
        .dht(false)
        .dht_bootstrap_nodes(Vec::new())
        .pex(false)
        .lsd(false)
        .max_connections(args.max_connections)
        .max_peers_per_torrent(args.max_peers)
        .unchoke_slots(args.unchoke_slots)
        .hash_threads(args.hash_threads)
        .upload_limit(args.upload_limit)
        .download_limit(args.download_limit)
        .build()
        .await
        .context("urtorrent could not start; check io_uring availability")?;

    let trial_result = async {
        let id: TorrentId = session
            .add_torrent(
                AddTorrent::metainfo(torrent_bytes, &args.save_path)
                    .paused(true)
                    .max_peers(args.max_peers)
                    .upload_limit(args.upload_limit)
                    .download_limit(args.download_limit)
                    .hold_after_metadata(true),
            )
            .await
            .context("urtorrent could not add the pinned fixture")?;
        session
            .release(id)
            .await
            .context("urtorrent could not prepare and check the paused fixture")?;

        let initial = session.status(id).await.context("cannot read initial torrent status")?;
        if initial.state != TorrentState::Paused {
            bail!("fixture must be prepared and paused before the start barrier");
        }
        if !initial.private {
            bail!("benchmark fixture must be marked private to disable peer discovery");
        }
        if args.mode == Mode::Seed && !initial.complete {
            bail!("seed mode requires a fully verified payload before the start barrier");
        }
        for tracker in session.trackers(id).await.context("cannot inspect fixture trackers")? {
            session
                .remove_tracker(id, tracker.url)
                .await
                .context("cannot remove tracker from isolated benchmark fixture")?;
        }
        for web_seed in initial.web_seed_urls {
            session
                .remove_web_seed(id, web_seed)
                .await
                .context("cannot remove web seed from isolated benchmark fixture")?;
        }
        let observed = session.status(id).await.context("cannot read isolated fixture status")?;
        if observed.state != TorrentState::Paused {
            bail!("fixture must remain paused until the common start barrier is released");
        }
        if observed.trackers_count != 0 || observed.web_seeds != 0 {
            bail!("benchmark fixture still has a tracker or web seed after isolation");
        }
        let settings = json!({
            "session": session_settings(&session.settings().await.context("cannot read effective session settings")?),
            "torrent": {
                "download_limit": observed.download_limit,
                "upload_limit": observed.upload_limit,
                "max_peers": observed.max_peers,
                "max_uploads": observed.max_uploads,
                "sequential": observed.sequential,
                "private": observed.private,
                "complete_before_start": observed.complete,
            },
            "control": {
                "dht_bootstrap_nodes": [],
                "peer_exchange": false,
                "local_service_discovery": false,
                "trackers_after_isolation": observed.trackers_count,
                "web_seeds_after_isolation": observed.web_seeds,
                "manual_peer": args.peer.to_string(),
                "start_barrier": true,
            },
            "runner_inputs": {
                "mode": match args.mode { Mode::Download => "download", Mode::Seed => "seed" },
                "timeout_seconds": args.timeout_seconds,
                "seed_seconds": args.seed_seconds,
                "hash_threads": args.hash_threads,
                "hash_threads_source": "command_line"
            }
        });
        write_identity_files(&args, settings)?;
        write_new_private(&args.ready_file, b"ready\n")?;

        let ready_deadline = Instant::now() + Duration::from_secs(args.timeout_seconds);
        wait_for_gate(&args.start_gate, ready_deadline).await?;
        let barrier_released_at = Instant::now();
        session.resume(id).await.context("urtorrent could not resume after the start barrier")?;
        session
            .add_peer(id, args.peer)
            .await
            .context("urtorrent could not add the local fixture peer")?;

        let deadline = barrier_released_at + Duration::from_secs(args.timeout_seconds);
        let mut first_complete_at = None;
        let final_status = loop {
            let status = session.status(id).await.context("cannot read torrent status")?;
            if status.state == TorrentState::Error {
                bail!("urtorrent stopped with error: {}", status.error.as_deref().unwrap_or("unknown"));
            }
            if status.complete {
                first_complete_at.get_or_insert_with(Instant::now);
                if args.mode == Mode::Download {
                    break status;
                }
                match fs::symlink_metadata(&args.stop_file) {
                    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                        break status;
                    }
                    Ok(_) => bail!("stop marker must be a regular non-symlink file"),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("cannot inspect trial stop marker"),
                }
                if first_complete_at
                    .is_some_and(|start| start.elapsed() >= Duration::from_secs(args.seed_seconds))
                {
                    break status;
                }
            }
            if Instant::now() >= deadline {
                bail!("urtorrent transfer exceeded the pinned timeout");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        };
        Ok::<_, anyhow::Error>((final_status, barrier_released_at.elapsed()))
    }
    .await;
    let shutdown_result = session
        .shutdown()
        .await
        .context("urtorrent shutdown failed");
    let (final_status, barrier_to_trial_end) = trial_result?;
    shutdown_result?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "format": "transfer-urtorrent-run-result-v1",
            "mode": match args.mode { Mode::Download => "download", Mode::Seed => "seed" },
            "complete": final_status.complete,
            "downloaded": final_status.downloaded,
            "uploaded": final_status.uploaded,
            "corrupt": final_status.corrupt,
            "redundant": final_status.redundant,
            "peers": final_status.peers,
            "barrier_to_trial_end_ms": barrier_to_trial_end.as_millis(),
        }))?
    );
    Ok(())
}

async fn run_preflight(values: Vec<String>) -> anyhow::Result<()> {
    let (torrent_path, save_path) = parse_preflight_args(values)?;
    let torrent_metadata = fs::symlink_metadata(&torrent_path).with_context(|| {
        format!(
            "cannot inspect preflight torrent {}",
            torrent_path.display()
        )
    })?;
    if torrent_metadata.file_type().is_symlink() || !torrent_metadata.is_file() {
        bail!("preflight torrent must be a regular non-symlink file");
    }
    if torrent_metadata.len() > 8 * 1024 * 1024 {
        bail!("preflight torrent exceeds the 8 MiB limit");
    }
    let torrent_bytes = fs::read(&torrent_path).context("cannot read preflight metainfo")?;
    let metainfo = urtorrent::Torrent::parse(&torrent_bytes)
        .context("urtorrent could not parse preflight metainfo")?;
    if metainfo.info.has_v2 {
        bail!("urtorrent preflight supports private v1 metainfo only");
    }
    if !metainfo.info.private {
        bail!("preflight torrent must carry the private flag");
    }

    let save_metadata = fs::symlink_metadata(&save_path).with_context(|| {
        format!(
            "cannot inspect preflight save directory {}",
            save_path.display()
        )
    })?;
    if save_metadata.file_type().is_symlink() || !save_metadata.is_dir() {
        bail!("preflight save path must be an existing non-symlink directory");
    }
    if fs::read_dir(&save_path)
        .context("cannot inspect preflight save directory")?
        .next()
        .is_some()
    {
        bail!("preflight save directory must be empty");
    }

    let session = Session::builder()
        .listen_port(0)
        .listen_v4(Some(Ipv4Addr::LOCALHOST))
        .listen_v6(None)
        .dht(false)
        .dht_bootstrap_nodes(Vec::new())
        .pex(false)
        .lsd(false)
        .max_connections(1)
        .max_peers_per_torrent(1)
        .hash_threads(1)
        .build()
        .await
        .context("urtorrent could not start the isolated preflight session")?;

    let mut added_id = None;
    let setup_result = async {
        let id = session
            .add_torrent(
                AddTorrent::metainfo(torrent_bytes, &save_path)
                    .paused(true)
                    .max_peers(1)
                    .hold_after_metadata(true),
            )
            .await
            .context("urtorrent rejected preflight metainfo")?;
        added_id = Some(id);
        session
            .release(id)
            .await
            .context("urtorrent could not prepare the paused preflight torrent")?;
        let status = session
            .status(id)
            .await
            .context("urtorrent could not read preflight torrent status")?;
        if status.state != TorrentState::Paused {
            bail!("urtorrent preflight torrent did not remain paused");
        }
        if !status.private {
            bail!("urtorrent preflight torrent lost its private flag");
        }
        let tracker_urls = session
            .trackers(id)
            .await
            .context("cannot inspect urtorrent preflight trackers")?
            .into_iter()
            .map(|tracker| tracker.url)
            .collect::<Vec<_>>();
        if tracker_urls.len() != 1 || tracker_urls[0] != "http://127.0.0.1:9/announce" {
            bail!("urtorrent preflight tracker is not the pinned loopback endpoint");
        }
        if !status.web_seed_urls.is_empty() {
            bail!("urtorrent preflight must not contain web seeds");
        }
        let effective_session = session
            .settings()
            .await
            .context("cannot read urtorrent preflight session settings")?;
        Ok::<_, anyhow::Error>(json!({
            "session": session_settings(&effective_session),
            "runner": {
                "hash_threads": 1,
                "dht_bootstrap_nodes": [],
            },
            "probe": {
                "paused": status.state == TorrentState::Paused,
                "private": status.private,
                "max_peers": status.max_peers,
                "tracker_urls": tracker_urls,
                "web_seed_urls": status.web_seed_urls,
            },
        }))
    }
    .await;
    let removal_result = if let Some(id) = added_id {
        session
            .remove_torrent_with_files(id)
            .await
            .context("urtorrent could not remove preflight torrent and payload")
    } else {
        Ok(())
    };
    let shutdown = session
        .shutdown()
        .await
        .context("urtorrent preflight shutdown failed");
    let settings = setup_result?;
    removal_result?;
    shutdown?;
    fs::remove_dir(&save_path).context("preflight save directory was not empty after removal")?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "format": "transfer-urtorrent-preflight-v1",
            "version": urtorrent::VERSION,
            "added": true,
            "paused": true,
            "removed": true,
            "settings": settings,
        }))?
    );
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let mut values = std::env::args().skip(1);
    if values.next().as_deref() == Some("preflight") {
        let values: Vec<_> = values.collect();
        if values.first().is_some_and(|value| is_help_flag(value)) {
            println!("{}", usage());
            Ok(())
        } else {
            run_preflight(values).await
        }
    } else {
        run_trial().await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("urtorrent-runner: {error:#}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_help_flag, is_task_local_ip, parse_preflight_args, parse_trial_args, Mode,
    };
    use std::net::Ipv4Addr;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[test]
    fn help_flags_are_recognized_without_a_value() {
        assert!(is_help_flag("--help"));
        assert!(is_help_flag("-h"));
        assert!(!is_help_flag("--mode"));
    }

    #[test]
    fn preflight_requires_distinct_absolute_paths() {
        let parsed = parse_preflight_args([
            "--torrent".to_owned(),
            "/tmp/probe.torrent".to_owned(),
            "--save-path".to_owned(),
            "/tmp/probe-data".to_owned(),
        ])
        .expect("valid preflight paths");
        assert_eq!(
            parsed,
            (
                PathBuf::from("/tmp/probe.torrent"),
                PathBuf::from("/tmp/probe-data")
            )
        );
        assert!(
            parse_preflight_args([
                "--torrent".to_owned(),
                "relative.torrent".to_owned(),
                "--save-path".to_owned(),
                "/tmp/probe-data".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse_preflight_args([
                "--torrent".to_owned(),
                "/tmp/first.torrent".to_owned(),
                "--torrent".to_owned(),
                "/tmp/second.torrent".to_owned(),
                "--save-path".to_owned(),
                "/tmp/probe-data".to_owned(),
            ])
            .is_err()
        );
    }

    fn valid_trial_args() -> Vec<String> {
        [
            "--mode",
            "download",
            "--namespace",
            "tngt-unit",
            "--torrent",
            "/tmp/probe.torrent",
            "--save-path",
            "/tmp/probe-data",
            "--pid-file",
            "/tmp/probe.pid",
            "--settings-file",
            "/tmp/probe.settings",
            "--start-gate",
            "/tmp/probe.gate",
            "--ready-file",
            "/tmp/probe.ready",
            "--stop-file",
            "/tmp/probe.stop",
            "--listen-v4",
            "10.0.0.2",
            "--peer",
            "10.0.0.3:5000",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    fn replace_value(args: &mut [String], name: &str, value: &str) {
        let index = args
            .iter()
            .position(|argument| argument == name)
            .expect("argument exists");
        args[index + 1] = value.to_owned();
    }

    #[test]
    fn trial_parser_accepts_the_isolated_private_v1_profile() {
        let parsed = parse_trial_args(valid_trial_args()).expect("valid trial profile");
        assert_eq!(parsed.mode, Mode::Download);
        assert_eq!(parsed.namespace, "tngt-unit");
        assert_eq!(parsed.listen_v4, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(parsed.peer.to_string(), "10.0.0.3:5000");
    }

    #[test]
    fn trial_parser_rejects_duplicate_options_and_colliding_control_paths() {
        let mut repeated = valid_trial_args();
        repeated.extend(["--mode".to_owned(), "seed".to_owned()]);
        assert!(parse_trial_args(repeated).is_err());

        let mut colliding_paths = valid_trial_args();
        replace_value(&mut colliding_paths, "--ready-file", "/tmp/probe.pid");
        assert!(parse_trial_args(colliding_paths).is_err());
    }

    #[test]
    fn trial_parser_rejects_public_addresses_invalid_namespace_and_bad_seed_window() {
        let mut public_listener = valid_trial_args();
        replace_value(&mut public_listener, "--listen-v4", "8.8.8.8");
        assert!(parse_trial_args(public_listener).is_err());

        let mut public_peer = valid_trial_args();
        replace_value(&mut public_peer, "--peer", "8.8.8.8:5000");
        assert!(parse_trial_args(public_peer).is_err());

        let mut invalid_namespace = valid_trial_args();
        replace_value(&mut invalid_namespace, "--namespace", "systemd-resolved");
        assert!(parse_trial_args(invalid_namespace).is_err());

        let mut invalid_seed_window = valid_trial_args();
        replace_value(&mut invalid_seed_window, "--mode", "seed");
        invalid_seed_window.extend([
            "--timeout-seconds".to_owned(),
            "10".to_owned(),
            "--seed-seconds".to_owned(),
            "11".to_owned(),
        ]);
        assert!(parse_trial_args(invalid_seed_window).is_err());
    }

    #[test]
    fn task_peer_policy_accepts_private_and_loopback_ranges_only() {
        for address in [
            "127.0.0.1",
            "10.2.3.4",
            "172.20.0.8",
            "192.168.4.9",
            "169.254.2.3",
        ] {
            assert!(is_task_local_ip(
                Ipv4Addr::from_str(address).expect("valid test IP")
            ));
        }
        for address in ["8.8.8.8", "100.96.0.2", "203.0.113.9"] {
            assert!(!is_task_local_ip(
                Ipv4Addr::from_str(address).expect("valid test IP")
            ));
        }
    }
}
