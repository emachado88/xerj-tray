//! Runtime configuration. Every path, timeout and command can be overridden
//! with an environment variable so the tray can be exercised headlessly
//! against stubs (see README "Testing").

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    /// systemd user unit that owns the daemon.
    pub unit: String,
    /// Path to the xerj binary (used for `xerj -V`).
    pub xerj_bin: PathBuf,
    /// Loopback probe host/port.
    pub host: String,
    pub port: u16,
    /// Web console URL opened by the menu item.
    pub console_url: String,
    /// Shell command that performs the binary update.
    pub update_cmd: String,
    /// GitHub "latest release" API endpoint used for the update pre-check.
    pub releases_api: String,
    /// Log file (truncated on every start).
    pub log_path: PathBuf,
    pub poll_interval: Duration,
    pub check_ttl: Duration,
    pub start_timeout: Duration,
    pub stop_timeout: Duration,
    pub update_timeout: Duration,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn env_string(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        let port = env_u64("XERJ_TRAY_PORT", 9200).clamp(1, 65535) as u16;
        let xerj_bin = match std::env::var_os("XERJ_TRAY_XERJ_BIN") {
            Some(p) => PathBuf::from(p),
            None => home().join(".local/bin/xerj"),
        };
        let log_path = match std::env::var_os("XERJ_TRAY_LOG") {
            Some(p) => PathBuf::from(p),
            None => {
                let state_dir = std::env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home().join(".local/state"));
                state_dir.join("xerj-tray").join("xerj-tray.log")
            }
        };
        Self {
            unit: env_string("XERJ_TRAY_UNIT", "xerj"),
            xerj_bin,
            host: env_string("XERJ_TRAY_HOST", "127.0.0.1"),
            port,
            console_url: format!("http://localhost:{port}/_xerj-console/"),
            update_cmd: env_string(
                "XERJ_TRAY_UPDATE_CMD",
                "curl -fsSL https://xerj.org/get | sh",
            ),
            releases_api: env_string(
                "XERJ_TRAY_RELEASES_API",
                "https://api.github.com/repos/xerj-org/xerj/releases/latest",
            ),
            log_path,
            poll_interval: Duration::from_millis(env_u64("XERJ_TRAY_POLL_MS", 2000).max(250)),
            check_ttl: Duration::from_secs(env_u64("XERJ_TRAY_CHECK_TTL_S", 3600)),
            start_timeout: Duration::from_secs(env_u64("XERJ_TRAY_START_TIMEOUT_S", 30)),
            stop_timeout: Duration::from_secs(env_u64("XERJ_TRAY_STOP_TIMEOUT_S", 60)),
            update_timeout: Duration::from_secs(env_u64("XERJ_TRAY_UPDATE_TIMEOUT_S", 900)),
        }
    }

    /// Human-readable endpoint used in tooltips and notifications.
    pub fn endpoint(&self) -> String {
        format!("localhost:{}", self.port)
    }
}
