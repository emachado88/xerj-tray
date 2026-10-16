# xerj-tray

System tray (StatusNotifierItem) controller for the XERJ search engine daemon on
Arch Linux / KDE Plasma 6 (Wayland).

Left click toggles the daemon, right click opens a menu with the live state, the
web console, the binary update and quit. Idle footprint is a few MB of RSS; the
~350 MB XERJ daemon is only resident while you actually want it.

## What it does

- **Left click** — start the daemon if stopped, stop it if running. Ignored
  while a transition or an update is in flight.
- **Right click** — menu:
  - `● XERJ: Running` / `○ XERJ: Stopped` / `◐ Starting…` / `◐ Stopping…` /
    `⟳ Updating…` — informational header (disabled).
  - `Start Server` / `Stop Server` — the same toggle.
  - `Open Web Console` — `xdg-open http://localhost:9200/_xerj-console/`,
    enabled only while something answers on the port.
  - `Update XERJ (v1.0.0-rc.96 → v1.0.0-rc.97)` — or `XERJ up to date (…)` when
    GitHub has nothing newer, `Checking for updates…` while the check runs and
    `Updating XERJ…` while the installer runs.
  - `Quit` — exits the tray only; the daemon keeps running.

The icon carries the state colour (green running, grey stopped, amber
transitioning, blue updating); the menu header carries the glyph, because
DBusMenu labels are plain text and cannot be coloured.

## Install

```sh
./install.sh        # build + binary + unit + autostart
./uninstall.sh      # undo it
```

`install.sh` builds the release binary, installs it to `~/.local/bin/xerj-tray`,
installs the systemd **user** unit `xerj.service` (the daemon) to
`~/.config/systemd/user/`, writes the desktop entry to both
`~/.local/share/applications/` (so it appears in the application launcher) and
`~/.config/autostart/` (so it starts with the session), and runs
`systemctl --user daemon-reload`.

The binary takes an advisory single-instance lock, so the launcher entry and the
session autostart can both fire — or you can double-click the launcher — without
putting a second icon in the panel: the extra launch logs and exits.

The daemon unit is **installed but not enabled**: enabling it would start the
350 MB daemon at every login, which is what the tray exists to avoid. To keep
the daemon always on instead, `systemctl --user enable xerj`.

## Layout

```
src/main.rs      start-up: config, logging, SNI registration, threads
src/tray.rs      the ksni Tray impl: icon, tooltip, click, menu
src/state.rs     the five lifecycle states, update sub-states, menu labels
src/actions.rs   poller + start/stop workers, in-flight transitions
src/update.rs    version pre-check (GitHub API) + installer run
src/system.rs    systemctl, TCP probe, `xerj -V`, notify-send, xdg-open
src/icons.rs     pixmap icons drawn in code (supersampled 4x)
packaging/       systemd unit + desktop entry template (launcher and autostart)
```

## Design notes

- **One tray per session.** An advisory `flock` on
  `$XDG_STATE_HOME/xerj-tray/instance.lock`: the launcher entry and the session
  autostart can both trigger without duplicating the panel icon.
- **No async runtime.** ksni's `blocking` feature drives its own D-Bus loop;
  the main thread calls `spawn()` and then `thread::park()`. A tokio runtime
  here would only add weight and a panicking `block_on`.
- **Never block the D-Bus thread.** `systemctl start/stop` and the installer
  run on worker threads; the menu and the click handler only mutate shared
  state and return. A `systemctl stop` can take up to `TimeoutStopSec`.
- **systemd is the source of truth** for the lifecycle; the port probe only
  disambiguates "starting" from "running" and detects a daemon started outside
  systemd (shown as `Running outside systemd`, which the tray will not kill).
- **Five states, with real transitions.** `Starting` is entered when a start is
  requested and left when the port answers (or when `XERJ_TRAY_START_TIMEOUT_S`
  expires, which raises a critical notification instead of lying).
- **No zombies.** Fire-and-forget children (`notify-send`, `xdg-open`) get a
  per-child reaper thread; result-collecting runs use `Command::output()`,
  which waits for its own child. `SIGCHLD` is never set to `SIG_IGN` — that
  makes `output()` fail with `ECHILD` after the child already ran.
- **Atomic updates.** `curl -fsSL https://xerj.org/get | sh` stages the new
  binary in the destination directory and renames it into place, so updating
  while the daemon runs is safe; the daemon is restarted afterwards to pick up
  the new inode.
- **Live menu.** `menu_about_to_show` is implemented, so state and version
  labels refresh on every open instead of sticking.

## Environment overrides

Everything is overridable, which is also how the tray is tested headlessly:

| Variable                            | Default                                                      |
| ----------------------------------- | ------------------------------------------------------------ |
| `XERJ_TRAY_UNIT`                    | `xerj`                                                       |
| `XERJ_TRAY_XERJ_BIN`                | `~/.local/bin/xerj`                                          |
| `XERJ_TRAY_HOST` / `XERJ_TRAY_PORT` | `127.0.0.1` / `9200`                                         |
| `XERJ_TRAY_UPDATE_CMD`              | `curl -fsSL https://xerj.org/get \| sh`                      |
| `XERJ_TRAY_RELEASES_API`            | `https://api.github.com/repos/xerj-org/xerj/releases/latest` |
| `XERJ_TRAY_LOG`                     | `$XDG_STATE_HOME/xerj-tray/xerj-tray.log`                    |
| `XERJ_TRAY_POLL_MS`                 | `2000`                                                       |
| `XERJ_TRAY_CHECK_TTL_S`             | `3600`                                                       |
| `XERJ_TRAY_START_TIMEOUT_S`         | `30`                                                         |
| `XERJ_TRAY_STOP_TIMEOUT_S`          | `60`                                                         |
| `XERJ_TRAY_UPDATE_TIMEOUT_S`        | `900`                                                        |

`install.sh` and `uninstall.sh` accept `BIN_DIR`, `UNIT_DIR`, `AUTOSTART_DIR`
and `UNIT_NAME`.

## Inspecting a running tray

```sh
# is the item registered?
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems

SVC=$(busctl --user list | grep StatusNotifierItem | awk '{print $1}' | head -n1)

# simulate a left click (toggle)
busctl --user call "$SVC" /StatusNotifierItem org.kde.StatusNotifierItem Activate ii 0 0

# menu tree, then click an item by id
busctl --user call "$SVC" /MenuBar com.canonical.dbusmenu GetLayout iias 0 5 0
busctl --user call "$SVC" /MenuBar com.canonical.dbusmenu Event isvu 3 clicked s "" 0
```

Menu item ids are reassigned on every rebuild, so re-fetch `GetLayout` and map
labels to ids right before clicking.

## Logs

`~/.local/state/xerj-tray/xerj-tray.log`, truncated on each start (the autostart
launch has no useful stderr).
