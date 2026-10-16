//! The state machine: a polling thread derives the daemon's state from systemd
//! and the API port, and the menu/click actions hand their blocking work to
//! worker threads so the D-Bus thread never stalls.
//!
//! INVARIANT: only the poller thread ever calls `HandleCell::refresh()`.
//! `Handle::update()` locks ksni's service mutex, which the D-Bus thread
//! already holds while it runs `activate`, `menu` or `menu_about_to_show` — so
//! calling it from inside a tray callback self-deadlocks and the host's method
//! call times out. Every other thread only mutates the shared state and sets
//! `dirty`.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use ksni::blocking::Handle;

use crate::config::Config;
use crate::logline;
use crate::state::{Pending, RunState, Snapshot, UpdateState};
use crate::system;
use crate::tray::XerjTray;

const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
/// How often the poller looks for a `dirty` flag. Full probes still run on
/// `poll_interval`; this only bounds how long a click takes to show up.
const TICK: Duration = Duration::from_millis(150);

/// The tray's D-Bus handle, stored once the service is up.
pub struct HandleCell {
    inner: OnceLock<Arc<Handle<XerjTray>>>,
}

impl HandleCell {
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    pub fn set(&self, handle: Arc<Handle<XerjTray>>) {
        let _ = self.inner.set(handle);
    }

    /// Push the shared state to the host. ksni only emits D-Bus signals for
    /// properties that actually changed, so calling this often is cheap.
    /// Poller thread only — see the module invariant.
    fn refresh(&self) {
        if let Some(handle) = self.inner.get() {
            let _ = handle.update(|_| {});
        }
    }

    fn is_closed(&self) -> bool {
        self.inner.get().map(|h| h.is_closed()).unwrap_or(false)
    }
}

impl Default for HandleCell {
    fn default() -> Self {
        Self::new()
    }
}

/// Lock helper that survives a poisoned mutex (a panicking worker must not
/// take the tray down).
pub fn lock(shared: &Mutex<Snapshot>) -> MutexGuard<'_, Snapshot> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ask the poller to push the current state to the host on its next tick.
pub fn mark_dirty(shared: &Mutex<Snapshot>) {
    lock(shared).dirty = true;
}

/// One initial probe before the tray registers, so the first icon and tooltip
/// are already correct.
pub fn initial_probe(cfg: &Config, shared: &Arc<Mutex<Snapshot>>) {
    let unit_active = system::unit_active(&cfg.unit).unwrap_or(false);
    let unit_present = system::unit_load_state(&cfg.unit)
        .map(|s| s != "not-found")
        .unwrap_or(true);
    let port_up = system::port_up(&cfg.host, cfg.port, PROBE_TIMEOUT);
    let version = system::xerj_version(&cfg.xerj_bin);

    let mut st = lock(shared);
    st.unit_active = unit_active;
    st.unit_present = unit_present;
    st.port_up = port_up;
    st.version = version;
    st.external_notified = port_up && !unit_active;
    st.state = if unit_active || port_up {
        RunState::Running
    } else {
        RunState::Stopped
    };
    st.refresh_icons();
    logline!(
        "initial probe: unit_active={unit_active} unit_present={unit_present} port_up={port_up} state={:?} version={}",
        st.state,
        st.version.clone().unwrap_or_else(|| "unknown".into())
    );
}

pub fn spawn_poller(cfg: Arc<Config>, shared: Arc<Mutex<Snapshot>>, cell: Arc<HandleCell>) {
    thread::spawn(move || {
        let mut next_poll = Instant::now();
        loop {
            if cell.is_closed() {
                logline!("tray service closed, stopping the poller");
                break;
            }
            let dirty = {
                let mut st = lock(&shared);
                let was = st.dirty;
                st.dirty = false;
                was
            };
            if Instant::now() >= next_poll {
                poll_once(&cfg, &shared);
                next_poll = Instant::now() + cfg.poll_interval;
                cell.refresh();
            } else if dirty {
                cell.refresh();
            }
            thread::sleep(TICK);
        }
    });
}

/// What the poll observed, to be reported outside the lock.
enum Effect {
    Started,
    Stopped,
    StartTimedOut,
    StopTimedOut,
    External,
    UpdateTimedOut,
}

fn poll_once(cfg: &Config, shared: &Arc<Mutex<Snapshot>>) {
    let unit_active = system::unit_active(&cfg.unit).unwrap_or(false);
    let load_state = system::unit_load_state(&cfg.unit);
    let port_up = system::port_up(&cfg.host, cfg.port, PROBE_TIMEOUT);
    let now = Instant::now();
    let mut effects: Vec<Effect> = Vec::new();

    {
        let mut st = lock(shared);
        st.unit_active = unit_active;
        st.port_up = port_up;
        if let Some(state) = load_state {
            st.unit_present = state != "not-found";
        }
        if st.version.is_none() {
            st.version = system::xerj_version(&cfg.xerj_bin);
        }

        // Resolve a lifecycle command that is in flight.
        match st.pending {
            Some(Pending::Start { deadline }) => {
                if st.port_up {
                    st.pending = None;
                    effects.push(Effect::Started);
                } else if now >= deadline {
                    st.pending = None;
                    effects.push(Effect::StartTimedOut);
                }
            }
            Some(Pending::Stop { deadline }) => {
                if !st.unit_active && !st.port_up {
                    st.pending = None;
                    effects.push(Effect::Stopped);
                } else if now >= deadline {
                    st.pending = None;
                    effects.push(Effect::StopTimedOut);
                }
            }
            None => {}
        }

        // An update that never returns must not wedge the entry forever.
        if matches!(st.update, UpdateState::Updating) {
            if let Some(started) = st.update_started {
                if now.duration_since(started) > cfg.update_timeout {
                    st.update_started = None;
                    st.update = UpdateState::Unknown;
                    // Invalidate the straggling run: it must not report success
                    // (or restart the daemon) after we gave up on it.
                    st.update_gen = st.update_gen.wrapping_add(1);
                    effects.push(Effect::UpdateTimedOut);
                }
            }
        }

        st.state = match st.pending {
            Some(Pending::Start { .. }) => RunState::Starting,
            Some(Pending::Stop { .. }) => RunState::Stopping,
            None if matches!(st.update, UpdateState::Updating) => RunState::Updating,
            None if st.unit_active || st.port_up => RunState::Running,
            None => RunState::Stopped,
        };

        if st.external() {
            if !st.external_notified {
                st.external_notified = true;
                effects.push(Effect::External);
            }
        } else {
            st.external_notified = false;
        }

        st.refresh_icons();
    }

    for effect in effects {
        match effect {
            Effect::Started => system::notify(
                "XERJ",
                &format!("Server started on {}", cfg.endpoint()),
                None,
                Some("system-search"),
            ),
            Effect::Stopped => system::notify(
                "XERJ",
                "Server stopped (memory freed)",
                None,
                Some("system-search"),
            ),
            Effect::StartTimedOut => system::notify(
                "XERJ",
                &format!(
                    "XERJ did not answer on {} within {}s — check the journal (systemctl --user status {})",
                    cfg.endpoint(),
                    cfg.start_timeout.as_secs(),
                    cfg.unit
                ),
                Some("critical"),
                Some("dialog-error"),
            ),
            Effect::StopTimedOut => system::notify(
                "XERJ",
                &format!(
                    "XERJ did not stop within {}s — it may be running outside systemd",
                    cfg.stop_timeout.as_secs()
                ),
                Some("critical"),
                Some("dialog-error"),
            ),
            Effect::External => system::notify(
                "XERJ",
                "XERJ is running outside systemd — the tray does not manage that process",
                Some("normal"),
                Some("dialog-warning"),
            ),
            Effect::UpdateTimedOut => system::notify(
                "XERJ",
                &format!("Update timed out after {}s", cfg.update_timeout.as_secs()),
                Some("critical"),
                Some("dialog-error"),
            ),
        }
    }
}

/// Left click: start when stopped, stop when running, ignored while
/// transitional (Starting/Stopping/Updating).
pub fn request_toggle(cfg: &Arc<Config>, shared: &Arc<Mutex<Snapshot>>) {
    let (state, unit_active) = {
        let st = lock(shared);
        (st.state, st.unit_active)
    };
    match state {
        RunState::Stopped => request_start(cfg, shared),
        RunState::Running if unit_active => request_stop(cfg, shared),
        RunState::Running => system::notify(
            "XERJ",
            "XERJ is running outside systemd — start it from the tray to manage it",
            Some("normal"),
            Some("dialog-warning"),
        ),
        RunState::Starting | RunState::Stopping | RunState::Updating => {
            logline!("toggle ignored while {state:?}")
        }
    }
}

pub fn request_start(cfg: &Arc<Config>, shared: &Arc<Mutex<Snapshot>>) {
    {
        let mut st = lock(shared);
        if !matches!(st.state, RunState::Stopped) || st.pending.is_some() {
            return;
        }
        if !st.unit_present {
            let unit = cfg.unit.clone();
            drop(st);
            system::notify(
                "XERJ",
                &format!("systemd user unit '{unit}' is not installed — run install.sh"),
                Some("critical"),
                Some("dialog-error"),
            );
            return;
        }
        st.pending = Some(Pending::Start {
            deadline: Instant::now() + cfg.start_timeout,
        });
        st.state = RunState::Starting;
        st.refresh_icons();
        st.dirty = true;
    }

    let (cfg, shared) = (cfg.clone(), shared.clone());
    thread::spawn(move || {
        if let Err(error) = system::systemctl("start", &cfg.unit) {
            {
                let mut st = lock(&shared);
                st.pending = None;
                st.state = RunState::Stopped;
                st.refresh_icons();
                st.dirty = true;
            }
            system::notify(
                "XERJ",
                &format!("Failed to start XERJ: {error}"),
                Some("critical"),
                Some("dialog-error"),
            );
        }
    });
}

pub fn request_stop(cfg: &Arc<Config>, shared: &Arc<Mutex<Snapshot>>) {
    {
        let mut st = lock(shared);
        if !matches!(st.state, RunState::Running) || st.pending.is_some() {
            return;
        }
        st.pending = Some(Pending::Stop {
            deadline: Instant::now() + cfg.stop_timeout,
        });
        st.state = RunState::Stopping;
        st.refresh_icons();
        st.dirty = true;
    }

    let (cfg, shared) = (cfg.clone(), shared.clone());
    thread::spawn(move || {
        if let Err(error) = system::systemctl("stop", &cfg.unit) {
            {
                let mut st = lock(&shared);
                st.pending = None;
                st.dirty = true;
            }
            system::notify(
                "XERJ",
                &format!("Failed to stop XERJ: {error}"),
                Some("critical"),
                Some("dialog-error"),
            );
        }
    });
}
