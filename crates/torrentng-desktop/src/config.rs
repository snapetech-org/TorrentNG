//! Persistent desktop settings (`~/.config/torrentng-desktop/config.toml`).
//!
//! Everything has a default so a missing or partial file never blocks startup;
//! unknown keys are tolerated so a downgrade does not lose the file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::theme::ThemeChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendMode {
    /// Spawn and supervise a bundled `torrentngd` on loopback.
    #[default]
    Local,
    /// Connect to an existing `torrentngd` (URL + API token).
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RemoteConfig {
    pub url: String,
    pub token: String,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8080".into(),
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LocalConfig {
    /// Explicit `torrentngd` path. Empty = next to this executable, then `PATH`.
    pub binary: String,
    /// Where the local engine keeps its session database. Empty = app data dir.
    pub data_dir: String,
    /// Where finished downloads go. Empty = `~/Downloads/TorrentNG`.
    pub download_dir: String,
    pub listen_port: u16,
    /// Start the engine when the app opens (otherwise use the toolbar button).
    pub autostart: bool,
    pub memory_cap_mb: u64,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            binary: String::new(),
            data_dir: String::new(),
            download_dir: String::new(),
            listen_port: 6881,
            autostart: true,
            memory_cap_mb: 256,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    pub mode: BackendMode,
    pub theme: ThemeChoice,
    pub local: LocalConfig,
    pub remote: RemoteConfig,
    pub show_sidebar: bool,
    pub show_detail: bool,
    /// Set once the first-run connection step has been completed.
    pub setup_done: bool,
}

impl Config {
    pub fn new() -> Self {
        Self {
            show_sidebar: true,
            show_detail: true,
            ..Self::default()
        }
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut c: Config = toml::from_str(text)?;
        // `Default` for bool is false; a file that omits these means "on".
        if !text.contains("show_sidebar") {
            c.show_sidebar = true;
        }
        if !text.contains("show_detail") {
            c.show_detail = true;
        }
        Ok(c)
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, path = %path.display(), "ignoring unreadable desktop config");
                Self::new()
            }),
            Err(_) => Self::new(),
        }
    }

    /// Atomic write (tmp + rename), mode 0600 on unix because it may hold a token.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_private(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, path)
    }
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

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Per-user config directory following each OS convention.
pub fn config_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("TORRENTNG_DESKTOP_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("TorrentNG Desktop");
    }
    #[cfg(target_os = "macos")]
    {
        return home().join("Library/Application Support/TorrentNG Desktop");
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("torrentng-desktop")
    }
}

/// Per-user data directory (local engine session state).
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("TORRENTNG_DESKTOP_DATA_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("TorrentNG Desktop");
    }
    #[cfg(target_os = "macos")]
    {
        return home().join("Library/Application Support/TorrentNG Desktop");
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
            .join("torrentng-desktop")
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn default_download_dir() -> PathBuf {
    home().join("Downloads").join("TorrentNG")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_file_missing_or_partial() {
        let c = Config::parse("").unwrap();
        assert_eq!(c.mode, BackendMode::Local);
        assert!(c.show_sidebar && c.show_detail);
        let c = Config::parse("mode = \"remote\"\n[remote]\nurl = \"http://nas:8080\"\n").unwrap();
        assert_eq!(c.mode, BackendMode::Remote);
        assert_eq!(c.remote.url, "http://nas:8080");
        assert!(c.remote.token.is_empty());
    }

    #[test]
    fn unknown_keys_are_tolerated() {
        assert!(Config::parse("future_option = 1\n").is_ok());
    }

    #[test]
    fn save_load_roundtrip_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let mut c = Config::new();
        c.mode = BackendMode::Remote;
        c.remote.token = "secret".into();
        c.theme = ThemeChoice::Light;
        c.show_detail = false;
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path), c);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "mode = [").unwrap();
        assert_eq!(Config::load(&path), Config::new());
    }
}
