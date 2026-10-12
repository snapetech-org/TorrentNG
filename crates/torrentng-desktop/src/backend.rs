//! Local mode: supervise a bundled `torrentngd` on loopback.
//!
//! The daemon is a normal child process (never in-process), so a crash in the
//! UI cannot corrupt torrent state and the engine's crash-safety guarantees
//! (`docs/CRASH_SAFETY.md`) are unchanged. It listens on `127.0.0.1` only,
//! on an ephemeral port, behind a random per-launch bearer token.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rand::RngCore;
use tokio::process::{Child, Command};

use crate::api::Client;
use crate::config::{self, LocalConfig};

const BIN_NAME: &str = if cfg!(windows) {
    "torrentngd.exe"
} else {
    "torrentngd"
};

pub struct LocalEngine {
    child: Child,
    pub url: String,
    pub token: String,
    pub log_path: PathBuf,
}

/// 256 bits of OS-seeded randomness, hex encoded.
pub fn new_token() -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Reserve a free loopback port. (Tiny race window; the daemon reports a
/// clear bind error in its log if someone grabs it.)
pub fn free_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// Locate `torrentngd`: explicit setting, then beside this executable (the
/// layout of every desktop bundle), then `PATH`.
pub fn find_binary(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        return p.is_file().then_some(p);
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        // Portable zip/tarball and macOS .app: engine sits beside the app.
        // Linux package: private copy in ../lib/torrentng-desktop so it never
        // collides with a separately installed `torrentngd` package.
        for p in [
            dir.join(BIN_NAME),
            dir.join("../lib/torrentng-desktop").join(BIN_NAME),
        ] {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(BIN_NAME))
            .find(|p| p.is_file())
    })
}

/// Render the daemon config for a loopback-only private engine.
pub fn render_config(
    local: &LocalConfig,
    port: u16,
    session_dir: &Path,
    download_dir: &Path,
    token: &str,
) -> String {
    use toml::Value;
    let mut daemon = toml::Table::new();
    daemon.insert(
        "api_bind".into(),
        Value::String(format!("127.0.0.1:{port}")),
    );
    daemon.insert(
        "session_dir".into(),
        Value::String(session_dir.display().to_string()),
    );
    daemon.insert("log_level".into(), Value::String("info".into()));
    let mut network = toml::Table::new();
    network.insert(
        "listen_port".into(),
        Value::Integer(local.listen_port.into()),
    );
    let mut storage = toml::Table::new();
    storage.insert(
        "download_dir".into(),
        Value::String(download_dir.display().to_string()),
    );
    let mut memory = toml::Table::new();
    memory.insert(
        "total_cap_mb".into(),
        Value::Integer(local.memory_cap_mb.max(64) as i64),
    );
    let mut auth = toml::Table::new();
    auth.insert(
        "api_tokens".into(),
        Value::Array(vec![Value::String(token.to_owned())]),
    );

    let mut root = toml::Table::new();
    root.insert("daemon".into(), Value::Table(daemon));
    root.insert("network".into(), Value::Table(network));
    root.insert("storage".into(), Value::Table(storage));
    root.insert("memory".into(), Value::Table(memory));
    root.insert("auth".into(), Value::Table(auth));
    toml::to_string_pretty(&root).expect("config serializes")
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

pub async fn start(local: &LocalConfig) -> Result<LocalEngine, String> {
    let bin = find_binary(&local.binary).ok_or_else(|| {
        "torrentngd was not found next to the app or on PATH. Set its location in Settings, \
         or switch to a remote server."
            .to_owned()
    })?;
    let data = if local.data_dir.trim().is_empty() {
        config::data_dir()
    } else {
        PathBuf::from(local.data_dir.trim())
    };
    let download = if local.download_dir.trim().is_empty() {
        config::default_download_dir()
    } else {
        PathBuf::from(local.download_dir.trim())
    };
    let session = data.join("engine");
    for d in [&session, &download] {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let port = free_port().map_err(|e| format!("no free loopback port: {e}"))?;
    let token = new_token();
    let cfg_path = data.join("engine-config.toml");
    write_private(
        &cfg_path,
        render_config(local, port, &session, &download, &token).as_bytes(),
    )
    .map_err(|e| format!("cannot write engine config: {e}"))?;

    let log_path = data.join("engine.log");
    let log =
        std::fs::File::create(&log_path).map_err(|e| format!("cannot open engine log: {e}"))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new(&bin)
        .env("TORRENTNGD_CONFIG", &cfg_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;

    let url = format!("http://127.0.0.1:{port}");
    let client = Client::new(&url, &token).map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "engine exited early ({status}):\n{}",
                tail(&log_path, 12)
            ));
        }
        if client.health().await.is_ok() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err(format!(
                "engine did not become ready in 30s:\n{}",
                tail(&log_path, 12)
            ));
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(LocalEngine {
        child,
        url,
        token,
        log_path,
    })
}

impl LocalEngine {
    /// Ask the daemon to shut down cleanly (it flushes fastresume state on
    /// SIGTERM), then force-kill after a grace period.
    pub async fn stop(mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status()
                .await;
        }
        #[cfg(not(unix))]
        let _ = self.child.start_kill();
        if tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .is_err()
        {
            tracing::warn!("engine ignored shutdown; killing");
            let _ = self.child.kill().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_unique() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rendered_config_is_loopback_only_and_valid_toml() {
        let local = LocalConfig::default();
        let text = render_config(
            &local,
            4242,
            Path::new("/s ession"),
            Path::new("/d\"l"),
            "tok",
        );
        let v: toml::Table = toml::from_str(&text).expect("valid toml even with odd paths");
        assert_eq!(v["daemon"]["api_bind"].as_str(), Some("127.0.0.1:4242"));
        assert_eq!(v["daemon"]["session_dir"].as_str(), Some("/s ession"));
        assert_eq!(v["storage"]["download_dir"].as_str(), Some("/d\"l"));
        assert_eq!(v["auth"]["api_tokens"][0].as_str(), Some("tok"));
        assert_eq!(v["network"]["listen_port"].as_integer(), Some(6881));
        assert!(!text.contains("0.0.0.0"));
    }

    #[test]
    fn memory_cap_has_a_floor() {
        let local = LocalConfig {
            memory_cap_mb: 1,
            ..Default::default()
        };
        let text = render_config(&local, 1, Path::new("/s"), Path::new("/d"), "t");
        let v: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(v["memory"]["total_cap_mb"].as_integer(), Some(64));
    }

    #[test]
    fn explicit_binary_must_exist() {
        assert!(find_binary("/definitely/not/here/torrentngd").is_none());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fake-torrentngd");
        std::fs::write(&p, b"").unwrap();
        assert_eq!(find_binary(p.to_str().unwrap()), Some(p));
    }

    #[test]
    fn free_port_is_usable() {
        let p = free_port().unwrap();
        assert!(p > 0);
        std::net::TcpListener::bind(("127.0.0.1", p)).expect("port is free again");
    }
}
