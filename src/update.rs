//! The update path: a cheap pre-check against the GitHub "latest release" API
//! (so the button does not re-download 140 MB for nothing) and the installer
//! run itself, both on detached worker threads.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::actions::{lock, mark_dirty};
use crate::config::Config;
use crate::logline;
use crate::state::{RunState, Snapshot, UpdateState};
use crate::system;

/// Read the `tag_name` of the latest release without a JSON dependency.
pub fn parse_tag_name(json: &str) -> Option<String> {
    let key = json.find("\"tag_name\"")?;
    let after_key = &json[key + "\"tag_name\"".len()..];
    let open = after_key.find('"')?;
    let after_open = &after_key[open + 1..];
    let close = after_open.find('"')?;
    let tag = after_open[..close].trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

fn check_latest(cfg: &Config) -> Result<String, String> {
    let out = system::capture(
        "curl",
        &[
            "-fsSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            &cfg.releases_api,
        ],
    )?;
    if !out.ok {
        return Err(system::error_text(&out));
    }
    parse_tag_name(&out.stdout).ok_or_else(|| "no tag_name in the release response".to_string())
}

/// Ask GitHub which version is current. `force` skips the TTL throttle; the
/// menu uses the throttled form on every open, the start-up uses the forced one.
pub fn spawn_check(cfg: &Arc<Config>, shared: &Arc<Mutex<Snapshot>>, force: bool) {
    {
        let mut st = lock(shared);
        if matches!(st.update, UpdateState::Checking | UpdateState::Updating) {
            return;
        }
        if !force {
            if let Some(last) = st.last_check {
                if last.elapsed() < cfg.check_ttl {
                    return;
                }
            }
        }
        st.update = UpdateState::Checking;
        st.last_check = Some(Instant::now());
        st.dirty = true;
    }

    let (cfg, shared) = (cfg.clone(), shared.clone());
    thread::spawn(move || {
        let latest = check_latest(&cfg);
        let current = {
            let mut st = lock(&shared);
            if st.version.is_none() {
                st.version = system::xerj_version(&cfg.xerj_bin);
            }
            st.version.clone()
        };
        if let Err(error) = &latest {
            logline!("update check failed: {error}");
        }
        let next = match (&latest, &current) {
            (Ok(latest), Some(current)) if current == latest => {
                UpdateState::UpToDate(current.clone())
            }
            (Ok(latest), Some(current)) => UpdateState::Available {
                current: current.clone(),
                latest: latest.clone(),
            },
            // No installed version to compare against (binary missing) or the
            // check failed: leave the entry usable as a forced install.
            _ => UpdateState::Unknown,
        };
        lock(&shared).update = next;
        mark_dirty(&shared);
    });
}

/// Run the installer. The install is an atomic rename into ~/.local/bin, so it
/// is safe while the daemon is running; the daemon is restarted afterwards so
/// it picks up the new binary.
pub fn spawn_update(cfg: &Arc<Config>, shared: &Arc<Mutex<Snapshot>>) {
    let generation = {
        let mut st = lock(shared);
        if !matches!(
            st.update,
            UpdateState::Available { .. } | UpdateState::Unknown
        ) {
            return;
        }
        st.update = UpdateState::Updating;
        st.update_started = Some(Instant::now());
        st.state = RunState::Updating;
        st.refresh_icons();
        st.dirty = true;
        st.update_gen = st.update_gen.wrapping_add(1);
        st.update_gen
    };

    let (cfg, shared) = (cfg.clone(), shared.clone());
    thread::spawn(move || {
        let was_running = system::unit_active(&cfg.unit).unwrap_or(false)
            || system::port_up(&cfg.host, cfg.port, std::time::Duration::from_millis(400));
        logline!(
            "update: running `{}` (was_running={was_running})",
            cfg.update_cmd
        );

        let outcome = system::capture_shell(&cfg.update_cmd);
        if lock(&shared).update_gen != generation {
            logline!("update: the run finished after the watchdog gave up on it — result ignored");
            mark_dirty(&shared);
            return;
        }
        match outcome {
            Ok(out) if out.ok => {
                let version = system::xerj_version(&cfg.xerj_bin);
                let shown = version
                    .clone()
                    .unwrap_or_else(|| "the latest version".to_string());
                logline!("update: installed {shown}");
                system::notify(
                    "XERJ",
                    &format!("Binary updated successfully to {shown}!"),
                    None,
                    Some("system-software-update"),
                );
                if was_running {
                    match system::systemctl("restart", &cfg.unit) {
                        Ok(()) => system::notify(
                            "XERJ",
                            &format!("Server restarted ({shown})"),
                            None,
                            Some("system-search"),
                        ),
                        Err(error) => system::notify(
                            "XERJ",
                            &format!("Binary updated, but the server did not restart: {error}"),
                            Some("critical"),
                            Some("dialog-error"),
                        ),
                    }
                }
                let mut st = lock(&shared);
                st.version = version.clone();
                st.update = match version {
                    Some(v) => UpdateState::UpToDate(v),
                    None => UpdateState::Unknown,
                };
                st.update_started = None;
            }
            Ok(out) => {
                let error = system::error_text(&out);
                logline!("update failed: {error}");
                finish_failed(&cfg, &shared, &error);
            }
            Err(error) => {
                logline!("update failed: {error}");
                finish_failed(&cfg, &shared, &error);
            }
        }
        mark_dirty(&shared);
    });
}

fn finish_failed(cfg: &Config, shared: &Arc<Mutex<Snapshot>>, error: &str) {
    {
        let mut st = lock(shared);
        st.update = UpdateState::Unknown;
        st.update_started = None;
        st.version = system::xerj_version(&cfg.xerj_bin).or_else(|| st.version.clone());
        st.dirty = true;
    }
    system::notify(
        "XERJ",
        &format!("Failed to update the binary: {error}"),
        Some("critical"),
        Some("dialog-error"),
    );
}
