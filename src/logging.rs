//! Minimal file logger.
//!
//! The tray is normally started by the Plasma autostart and its stderr goes
//! nowhere useful, so every diagnostic line lands in a log file instead.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();

// Civil date from a Unix day number (Howard Hinnant's algorithm). Kept local so
// the tray needs no date/time crate.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Create (truncating) the log file. A failure is not fatal: the tray runs
/// without logging rather than refusing to start.
pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .ok();
    let _ = LOG.set(Mutex::new(file));
}

pub fn log(message: &str) {
    let line = format!("[{}] {}\n", utc_timestamp(), message);
    if let Some(cell) = LOG.get() {
        if let Ok(mut guard) = cell.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    } else {
        eprint!("{line}");
    }
}

/// Append one line without truncating: used when a second launch is rejected
/// *before* the logger is initialised, so it cannot wipe the live instance's
/// log.
pub fn append(path: &Path, message: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{}] {}", utc_timestamp(), message);
    }
}

#[macro_export]
macro_rules! logline {
    ($($arg:tt)*) => {
        $crate::logging::log(&format!($($arg)*))
    };
}
