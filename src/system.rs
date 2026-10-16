//! Everything that talks to the outside world: systemctl, the TCP probe,
//! `xerj -V`, desktop notifications and URL opening.
//!
//! Child-process rules (see the README's design notes):
//! * fire-and-forget spawns get a per-child reaper thread;
//! * result-collecting runs use `output()`, which waits for its own child;
//! * never `signal(SIGCHLD, SIG_IGN)` — it makes `output()` fail with ECHILD.

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::logline;

pub struct CmdOut {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program and capture its output. `output()` closes the child's stdin,
/// so a command that unexpectedly prompts cannot hang the caller.
pub fn capture(program: &str, args: &[&str]) -> Result<CmdOut, String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    Ok(CmdOut {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Run a shell command line (`sh -c`, so pipes and redirections work).
pub fn capture_shell(command: &str) -> Result<CmdOut, String> {
    capture("sh", &["-c", command])
}

/// One short line describing a failure: stderr if it said anything, else stdout.
pub fn error_text(out: &CmdOut) -> String {
    let pick = |text: &str| -> Option<String> {
        text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .next_back()
            .map(|l| l.to_string())
    };
    let raw = pick(&out.stderr)
        .or_else(|| pick(&out.stdout))
        .unwrap_or_else(|| "unknown error".into());
    if raw.chars().count() > 200 {
        let mut s: String = raw.chars().take(200).collect();
        s.push('…');
        s
    } else {
        raw
    }
}

/// Fire-and-forget spawn, reaped by a dedicated thread (no zombies).
fn spawn_detached(program: &str, args: Vec<String>) {
    match Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => logline!("could not spawn {program}: {e}"),
    }
}

/// `systemctl --user <action> <unit>`, blocking. Call only from a worker thread.
pub fn systemctl(action: &str, unit: &str) -> Result<(), String> {
    let out = capture("systemctl", &["--user", action, unit])?;
    if out.ok {
        logline!("systemctl --user {action} {unit}: ok");
        Ok(())
    } else {
        let text = error_text(&out);
        logline!("systemctl --user {action} {unit}: {text}");
        Err(text)
    }
}

/// `Some(true)` active, `Some(false)` not active, `None` if systemctl itself
/// could not be run.
pub fn unit_active(unit: &str) -> Option<bool> {
    capture("systemctl", &["--user", "is-active", "--quiet", unit])
        .ok()
        .map(|o| o.ok)
}

/// LoadState of the unit, for telling "unit not installed" apart from
/// "unit installed but stopped".
pub fn unit_load_state(unit: &str) -> Option<String> {
    let out = capture(
        "systemctl",
        &["--user", "show", "--property=LoadState", "--value", unit],
    )
    .ok()?;
    let state = out.stdout.trim().to_string();
    if state.is_empty() { None } else { Some(state) }
}

/// Cheap liveness probe: a TCP connect to the loopback API port.
pub fn port_up(host: &str, port: u16, timeout: Duration) -> bool {
    let Ok(addr) = format!("{host}:{port}").parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Installed version, parsed out of `xerj -V` ("xerj v1.0.0-rc.96").
pub fn xerj_version(bin: &Path) -> Option<String> {
    let path = bin.to_str()?;
    let out = capture(path, &["-V"]).ok()?;
    if !out.ok {
        return None;
    }
    parse_version(&out.stdout)
}

pub fn parse_version(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let bare = token.trim_start_matches('v');
        let looks_numeric = bare
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
        if looks_numeric && bare.contains('.') {
            let cleaned: String = bare
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '+')
                .collect();
            if !cleaned.is_empty() {
                return Some(format!("v{cleaned}"));
            }
        }
    }
    None
}

pub fn notify(summary: &str, body: &str, urgency: Option<&str>, icon: Option<&str>) {
    let mut args: Vec<String> = vec!["-a".into(), "XERJ".into()];
    if let Some(u) = urgency {
        args.push("-u".into());
        args.push(u.into());
    }
    if let Some(i) = icon {
        args.push("-i".into());
        args.push(i.into());
    }
    args.push(summary.into());
    args.push(body.into());
    logline!("notify: {summary} — {body}");
    spawn_detached("notify-send", args);
}

pub fn open_url(url: &str) {
    logline!("open: {url}");
    spawn_detached("xdg-open", vec![url.to_string()]);
}
