//! xerj-tray — a zero-cost StatusNotifierItem controller for the XERJ daemon.
//!
//! No async runtime: ksni's `blocking` feature drives its own D-Bus loop, and
//! the main thread parks after registering the item.

mod actions;
mod config;
mod icons;
mod logging;
mod state;
mod system;
mod tray;
mod update;

use std::sync::{Arc, Mutex};

use ksni::blocking::TrayMethods;

use crate::config::Config;
use crate::state::Snapshot;

fn main() {
    let cfg = Arc::new(Config::from_env());

    // One tray per session: the launcher entry and the session autostart can
    // both be triggered, and a second registration would put a duplicate icon
    // in the panel. Claimed before the logger opens, so a rejected launch does
    // not truncate the live instance's log.
    if !claim_single_instance(&cfg) {
        eprintln!("xerj-tray: another instance is already running — exiting");
        logging::append(
            &cfg.log_path,
            "another xerj-tray instance is already running — exiting",
        );
        return;
    }

    logging::init(&cfg.log_path);
    logline!(
        "xerj-tray {} starting (unit={}, endpoint={}, xerj={})",
        env!("CARGO_PKG_VERSION"),
        cfg.unit,
        cfg.endpoint(),
        cfg.xerj_bin.display()
    );

    let shared = Arc::new(Mutex::new(Snapshot::initial()));
    let cell = Arc::new(actions::HandleCell::new());

    if system::unit_load_state(&cfg.unit).as_deref() == Some("not-found") {
        logline!("systemd user unit '{}' is not installed", cfg.unit);
        system::notify(
            "XERJ Tray",
            &format!(
                "systemd user unit '{}' not found — run install.sh to create it",
                cfg.unit
            ),
            Some("normal"),
            Some("dialog-warning"),
        );
    }

    actions::initial_probe(&cfg, &shared);

    let tray = tray::XerjTray::new(cfg.clone(), shared.clone());
    match tray.assume_sni_available(true).spawn() {
        Ok(handle) => {
            cell.set(Arc::new(handle));
            logline!("registered with the StatusNotifierWatcher");
        }
        Err(error) => {
            logline!("could not register the SNI item: {error}");
            system::notify(
                "XERJ Tray",
                &format!("Could not register the tray item: {error}"),
                Some("critical"),
                Some("dialog-error"),
            );
            std::process::exit(1);
        }
    }

    actions::spawn_poller(cfg.clone(), shared.clone(), cell.clone());
    update::spawn_check(&cfg, &shared, true);

    logline!("ready");
    loop {
        std::thread::park();
    }
}

/// Advisory single-instance lock next to the log. `try_lock` never blocks, and
/// the kernel drops the lock when the process dies — including on SIGKILL — so
/// the lock *file* can never be stale and block the next login.
fn claim_single_instance(cfg: &Config) -> bool {
    let dir = cfg
        .log_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let path = dir.join("instance.lock");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            logline!(
                "could not open {} ({error}) — continuing without the single-instance guard",
                path.display()
            );
            return true;
        }
    };
    match file.try_lock() {
        Ok(()) => {
            // Hold the descriptor (and the lock) for the whole process.
            std::mem::forget(file);
            true
        }
        Err(std::fs::TryLockError::WouldBlock) => false,
        Err(std::fs::TryLockError::Error(error)) => {
            eprintln!("xerj-tray: instance lock unavailable ({error}) — continuing anyway");
            true
        }
    }
}
