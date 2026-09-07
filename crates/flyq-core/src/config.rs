//! Persistent application configuration.
//!
//! LanChat keeps a small TOML file in the platform config directory holding the
//! user-adjustable settings: display nickname, download directory, and the
//! UDP/TCP port used for discovery and file transfer.
//!
//! The file is loaded once at startup and applied to the network stack. Changes
//! made in the settings panel are written back to disk; nickname and port are
//! baked into the discovery/transport sockets at launch, so they take effect on
//! the next start of the application.

use flyq_protocol::UserStatus;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default IPMsg/FeiQ port (UDP discovery + TCP file transfer).
pub const DEFAULT_PORT: u16 = 2425;

/// Supported UI locales.
pub const SUPPORTED_LOCALES: &[&str] = &["zh-CN", "en"];

/// Default UI locale.
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// Supported theme mode values.
pub const SUPPORTED_THEME_MODES: &[&str] = &["system", "light", "dark"];

/// Default theme mode (follows OS appearance).
pub const DEFAULT_THEME_MODE: &str = "system";

/// User-adjustable application settings, persisted as TOML.
///
/// `#[serde(default)]` makes the format forward/backward compatible: a config
/// file missing any field falls back to that field's default, so hand-edited or
/// older files still load cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Display name broadcast to peers.
    pub nickname: String,
    /// Directory where incoming files and folders are saved.
    pub download_dir: PathBuf,
    /// UDP/TCP port for discovery and file transfer.
    pub port: u16,
    /// Our presence status, broadcast to peers and restored on next launch.
    pub status: UserStatus,
    /// UI locale identifier (e.g. "zh-CN", "en").
    pub language: String,
    /// Whether to play a sound when a notification is raised.
    pub sound_enabled: bool,
    /// Theme mode: "system" (follow OS), "light", or "dark".
    pub theme_mode: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            nickname: default_nickname(),
            download_dir: default_download_dir(),
            port: DEFAULT_PORT,
            status: UserStatus::Online,
            language: DEFAULT_LOCALE.to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        }
    }
}

impl AppConfig {
    /// Absolute path of the config file on this platform.
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    /// Load the config from its default location, falling back to defaults on
    /// any read or parse error (a corrupt file must never prevent startup).
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// Load a config from an explicit path (exposed for testing).
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<AppConfig>(&text) {
                Ok(cfg) => {
                    tracing::info!("Loaded config from {:?}", path);
                    cfg
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {:?}: {}; using defaults", path, e);
                    Self::default()
                }
            },
            Err(_) => {
                tracing::info!("No config at {:?}; using defaults", path);
                Self::default()
            }
        }
    }

    /// Persist the config to its default location, creating the directory tree
    /// if needed.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    /// Persist the config to an explicit path (exposed for testing).
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)?;
        tracing::info!("Saved config to {:?}", path);
        Ok(())
    }

    /// Sanitize user input in place: trim the nickname and fall back to the
    /// hostname when it is blank, and clamp an invalid port to the default.
    ///
    /// Returns true when any value was corrected, so callers can decide whether
    /// to inform the user.
    pub fn normalize(&mut self) -> bool {
        let mut changed = false;

        let trimmed = self.nickname.trim();
        if trimmed.is_empty() {
            self.nickname = default_nickname();
            changed = true;
        } else if trimmed != self.nickname {
            self.nickname = trimmed.to_string();
            changed = true;
        }

        if self.port == 0 {
            self.port = DEFAULT_PORT;
            changed = true;
        }

        if self.download_dir.as_os_str().is_empty() {
            self.download_dir = default_download_dir();
            changed = true;
        }

        if !SUPPORTED_LOCALES.contains(&self.language.as_str()) {
            self.language = DEFAULT_LOCALE.to_string();
            changed = true;
        }

        if !SUPPORTED_THEME_MODES.contains(&self.theme_mode.as_str()) {
            self.theme_mode = DEFAULT_THEME_MODE.to_string();
            changed = true;
        }

        changed
    }
}

/// The default display name: the machine hostname, or a generic fallback.
pub fn default_nickname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "LanChat User".to_string())
}

/// The default download directory: `<home>/Downloads/LanChat`, falling back to
/// a relative `downloads` folder when the home directory can't be determined.
pub fn default_download_dir() -> PathBuf {
    match home_dir() {
        Some(h) => h.join("Downloads").join("LanChat"),
        None => PathBuf::from("downloads"),
    }
}

/// Best-effort home directory resolution without external crates.
fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    } else {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

/// The platform-specific configuration directory, ending in `LanChat`.
///
/// - Windows: `%APPDATA%\LanChat`
/// - macOS: `~/Library/Application Support/LanChat`
/// - Linux/other: `$XDG_CONFIG_HOME/LanChat` or `~/.config/LanChat`
pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("LanChat")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "flyq-config-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn default_should_use_ipmsg_port() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.port, DEFAULT_PORT);
        assert!(!cfg.nickname.trim().is_empty());
        assert!(!cfg.download_dir.as_os_str().is_empty());
    }

    #[test]
    fn save_then_load_should_roundtrip() {
        let dir = unique_temp_path("roundtrip");
        let path = dir.join("config.toml");

        let cfg = AppConfig {
            nickname: "Alice".to_string(),
            download_dir: PathBuf::from("/tmp/files"),
            port: 3000,
            status: UserStatus::Away,
            language: "en".to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        cfg.save_to(&path).expect("save should succeed");

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.status, UserStatus::Away);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_from_missing_file_should_return_defaults() {
        let path = unique_temp_path("missing").join("config.toml");
        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.port, DEFAULT_PORT);
        assert_eq!(loaded.download_dir, default_download_dir());
    }

    #[test]
    fn partial_file_should_fill_missing_fields_with_defaults() {
        let dir = unique_temp_path("partial");
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "port = 4000\n").expect("write partial config");

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.port, 4000);
        assert_eq!(loaded.nickname, default_nickname());
        assert_eq!(loaded.download_dir, default_download_dir());
        assert_eq!(loaded.status, UserStatus::Online);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_from_corrupt_file_should_return_defaults() {
        let dir = unique_temp_path("corrupt");
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "this is = not = valid toml [[[")
            .expect("write corrupt config");

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.port, DEFAULT_PORT);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_should_trim_blank_nickname_and_fix_zero_port() {
        let mut cfg = AppConfig {
            nickname: "   ".to_string(),
            download_dir: PathBuf::from("/keep"),
            port: 0,
            status: UserStatus::Online,
            language: DEFAULT_LOCALE.to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        let changed = cfg.normalize();
        assert!(changed);
        assert_eq!(cfg.nickname, default_nickname());
        assert_eq!(cfg.port, DEFAULT_PORT);
        assert_eq!(cfg.download_dir, PathBuf::from("/keep"));
    }

    #[test]
    fn normalize_should_trim_surrounding_whitespace() {
        let mut cfg = AppConfig {
            nickname: "  Bob  ".to_string(),
            download_dir: PathBuf::from("/d"),
            port: 2425,
            status: UserStatus::Online,
            language: DEFAULT_LOCALE.to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        let changed = cfg.normalize();
        assert!(changed);
        assert_eq!(cfg.nickname, "Bob");
    }

    #[test]
    fn normalize_should_report_no_change_for_clean_config() {
        let mut cfg = AppConfig {
            nickname: "Carol".to_string(),
            download_dir: PathBuf::from("/d"),
            port: 2425,
            status: UserStatus::Online,
            language: DEFAULT_LOCALE.to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        assert!(!cfg.normalize());
    }

    #[test]
    fn config_dir_should_end_with_lanchat() {
        let dir = config_dir();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("LanChat"));
    }

    #[test]
    fn normalize_should_clamp_unsupported_language_to_default() {
        let mut cfg = AppConfig {
            nickname: "Alice".to_string(),
            download_dir: PathBuf::from("/d"),
            port: 2425,
            status: UserStatus::Online,
            language: "ja-JP".to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        assert!(cfg.normalize());
        assert_eq!(cfg.language, DEFAULT_LOCALE);

        // Supported locale should pass through unchanged.
        let mut cfg2 = AppConfig {
            nickname: "Bob".to_string(),
            download_dir: PathBuf::from("/d"),
            port: 2425,
            status: UserStatus::Online,
            language: "en".to_string(),
            sound_enabled: true,
            theme_mode: DEFAULT_THEME_MODE.to_string(),
        };
        assert!(!cfg2.normalize());
        assert_eq!(cfg2.language, "en");
    }

    #[test]
    fn normalize_should_clamp_unsupported_theme_mode() {
        let mut cfg = AppConfig {
            nickname: "Alice".to_string(),
            download_dir: PathBuf::from("/d"),
            port: 2425,
            status: UserStatus::Online,
            language: DEFAULT_LOCALE.to_string(),
            sound_enabled: true,
            theme_mode: "neon".to_string(),
        };
        assert!(cfg.normalize());
        assert_eq!(cfg.theme_mode, DEFAULT_THEME_MODE);

        // Valid values should pass through unchanged.
        for mode in SUPPORTED_THEME_MODES {
            let mut c = AppConfig {
                theme_mode: mode.to_string(),
                ..AppConfig::default()
            };
            let changed = c.normalize();
            // Only report change if the mode differs from default
            if *mode == DEFAULT_THEME_MODE {
                assert!(!changed, "default mode should not trigger change");
            }
            assert_eq!(c.theme_mode, *mode);
        }
    }
}
