//! The StatusNotifierItem surface: icon, tooltip, left-click and the menu.
//!
//! Everything here runs on ksni's D-Bus thread, so no callback may block or
//! call `Handle::update()`. They only mutate the shared state (and set
//! `dirty`), and the poller thread publishes the change.

use std::sync::{Arc, Mutex};

use ksni::{Category, Icon, MenuItem, ToolTip, Tray};

use crate::actions;
use crate::config::Config;
use crate::logline;
use crate::state::Snapshot;
use crate::system;
use crate::update;

pub struct XerjTray {
    pub cfg: Arc<Config>,
    pub shared: Arc<Mutex<Snapshot>>,
}

impl XerjTray {
    pub fn new(cfg: Arc<Config>, shared: Arc<Mutex<Snapshot>>) -> Self {
        Self { cfg, shared }
    }
}

impl Tray for XerjTray {
    /// Must stay false: with it true Plasma opens the menu on a left click
    /// instead of calling `activate`, and the click should toggle the server.
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "xerj-tray".into()
    }

    fn title(&self) -> String {
        "XERJ".into()
    }

    fn category(&self) -> Category {
        Category::SystemServices
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        actions::lock(&self.shared).icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        let snapshot = actions::lock(&self.shared);
        ToolTip {
            title: "XERJ".into(),
            description: snapshot.tooltip(&self.cfg.endpoint()),
            ..Default::default()
        }
    }

    /// Required for a live menu: ksni reads the trait's default impl as "not
    /// implemented" and skips the refresh, leaving stale labels behind.
    /// Also the natural moment to re-check for a newer release (throttled).
    fn menu_about_to_show(&mut self) {
        update::spawn_check(&self.cfg, &self.shared, false);
    }

    /// Session autostart race: Plasma's watcher may not be up yet. Wait for it
    /// instead of shutting the tray down.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        logline!("StatusNotifierWatcher offline ({reason:?}) — waiting for it to come back");
        true
    }

    /// Left click: toggle the server.
    fn activate(&mut self, _x: i32, _y: i32) {
        actions::request_toggle(&self.cfg, &self.shared);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        use ksni::menu::*;

        let snapshot = actions::lock(&self.shared).clone();
        let (update_label, update_enabled) = snapshot.update_entry();

        vec![
            // Informational header. DBusMenu labels are plain text, so the
            // colour signal lives in the item icon and the tray pixmap.
            StandardItem {
                label: snapshot.header_label(),
                icon_name: snapshot.header_icon(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: snapshot.toggle_label(),
                enabled: snapshot.toggle_enabled(),
                icon_name: snapshot.toggle_icon(),
                activate: Box::new(|tray: &mut Self| {
                    actions::request_toggle(&tray.cfg, &tray.shared);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open Web Console".into(),
                enabled: snapshot.console_enabled(),
                icon_name: "internet-web-browser".into(),
                activate: Box::new(|tray: &mut Self| {
                    let url = tray.cfg.console_url.clone();
                    std::thread::spawn(move || system::open_url(&url));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: update_label,
                enabled: update_enabled,
                icon_name: "system-software-update".into(),
                activate: Box::new(|tray: &mut Self| {
                    update::spawn_update(&tray.cfg, &tray.shared);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                // Leaves the daemon alone on purpose: quitting the tray is not
                // a request to stop the server.
                activate: Box::new(|_tray: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}
