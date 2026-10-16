//! Observable state of the daemon, shared between the D-Bus thread (which
//! renders it) and the worker threads (which mutate it).

use ksni::Icon;
use std::time::Instant;

/// The five lifecycle states surfaced in the icon, tooltip and menu header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunState {
    Stopped,
    /// `systemctl start` was issued; waiting for the port to answer.
    Starting,
    Running,
    /// `systemctl stop` was issued; waiting for the process to leave.
    Stopping,
    /// A binary update is in flight.
    Updating,
}

/// Sub-state of the single "update" menu entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpdateState {
    /// Not checked yet, or the check failed (offline): offer a forced install.
    Unknown,
    Checking,
    UpToDate(String),
    Available {
        current: String,
        latest: String,
    },
    Updating,
}

/// A lifecycle command that is currently in flight, with the moment we stop
/// waiting for it.
#[derive(Clone, Copy, Debug)]
pub enum Pending {
    Start { deadline: Instant },
    Stop { deadline: Instant },
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub state: RunState,
    /// The systemd unit is active.
    pub unit_active: bool,
    /// The unit exists at all (false = install.sh was never run).
    pub unit_present: bool,
    /// Something answers on the HTTP port.
    pub port_up: bool,
    pub version: Option<String>,
    pub update: UpdateState,
    pub pending: Option<Pending>,
    pub last_check: Option<Instant>,
    pub update_started: Option<Instant>,
    /// Whether we already told the user about a daemon started outside systemd.
    pub external_notified: bool,
    /// Cached tray icon (regenerated only when state or theme changes).
    pub icons: Vec<Icon>,
    pub icon_key: (RunState, bool),
    /// Set by any thread that changed something the host should re-read. Only
    /// the poller thread clears it and pushes the change (see `actions`).
    pub dirty: bool,
    /// Bumped on every update attempt and again when the watchdog cancels one,
    /// so a straggling installer run cannot report a result that was already
    /// abandoned.
    pub update_gen: u64,
}

impl Snapshot {
    pub fn initial() -> Self {
        Self {
            state: RunState::Stopped,
            unit_active: false,
            unit_present: true,
            port_up: false,
            version: None,
            update: UpdateState::Unknown,
            pending: None,
            last_check: None,
            update_started: None,
            external_notified: false,
            icons: Vec::new(),
            icon_key: (RunState::Stopped, true),
            dirty: false,
            update_gen: 0,
        }
    }

    /// Rebuild the pixmaps when the state or the panel tone changed.
    pub fn refresh_icons(&mut self) {
        let dark = crate::icons::theme_is_dark().unwrap_or(true);
        if self.icon_key != (self.state, dark) || self.icons.is_empty() {
            self.icons = crate::icons::build_icons(self.state, dark);
            self.icon_key = (self.state, dark);
        }
    }

    /// True when the daemon answers on the port but was not started by the
    /// systemd unit we control.
    pub fn external(&self) -> bool {
        self.port_up && !self.unit_active
    }

    /// Header text. The state marker is the item's icon, not a glyph: every
    /// other menu entry has an icon, so a bare "●" made this one look like the
    /// odd one out.
    pub fn header_label(&self) -> String {
        match self.state {
            RunState::Stopped => "XERJ: Stopped".into(),
            RunState::Starting => "XERJ: Starting…".into(),
            RunState::Running => "XERJ: Running".into(),
            RunState::Stopping => "XERJ: Stopping…".into(),
            RunState::Updating => "XERJ: Updating…".into(),
        }
    }

    /// System icon for the header. The theme's `emblem-*` family carries the
    /// same colour language as the tray pixmap (green / grey / amber), and an
    /// item icon *can* be coloured, unlike a DBusMenu label.
    pub fn header_icon(&self) -> String {
        match self.state {
            RunState::Running => "emblem-success",
            RunState::Stopped => "emblem-unavailable",
            RunState::Starting | RunState::Stopping => "emblem-warning",
            RunState::Updating => "emblem-synchronizing",
        }
        .into()
    }

    pub fn tooltip(&self, endpoint: &str) -> String {
        let text = match self.state {
            RunState::Stopped => "XERJ: Stopped".to_string(),
            RunState::Starting => format!("XERJ: Starting ({endpoint})"),
            RunState::Running => {
                if self.external() {
                    format!("XERJ: Running outside systemd ({endpoint})")
                } else if self.port_up {
                    format!("XERJ: Running ({endpoint})")
                } else {
                    format!("XERJ: Running ({endpoint} not responding)")
                }
            }
            RunState::Stopping => "XERJ: Stopping…".to_string(),
            RunState::Updating => "XERJ: Updating binary…".to_string(),
        };
        if self.unit_present {
            text
        } else {
            format!("{text}\nsystemd unit not installed — run install.sh")
        }
    }

    pub fn toggle_label(&self) -> String {
        match self.state {
            RunState::Stopped => "Start Server".into(),
            RunState::Starting => "Starting…".into(),
            RunState::Running => "Stop Server".into(),
            RunState::Stopping => "Stopping…".into(),
            RunState::Updating => "Updating…".into(),
        }
    }

    pub fn toggle_enabled(&self) -> bool {
        matches!(self.state, RunState::Stopped | RunState::Running)
    }

    pub fn toggle_icon(&self) -> String {
        match self.state {
            RunState::Running => "media-playback-stop".into(),
            _ => "media-playback-start".into(),
        }
    }

    /// The console is only useful once something answers on the port.
    pub fn console_enabled(&self) -> bool {
        self.port_up
    }

    /// Label and enabled flag of the update entry.
    pub fn update_entry(&self) -> (String, bool) {
        match &self.update {
            UpdateState::Checking => ("Checking for updates…".into(), false),
            UpdateState::Updating => ("Updating XERJ…".into(), false),
            UpdateState::UpToDate(v) => (format!("XERJ up to date ({v})"), false),
            UpdateState::Available { current, latest } => {
                (format!("Update XERJ ({current} → {latest})"), true)
            }
            UpdateState::Unknown => {
                let label = match &self.version {
                    Some(v) => format!("Update XERJ ({v})"),
                    None => "Update XERJ (latest)".to_string(),
                };
                (label, true)
            }
        }
    }
}
