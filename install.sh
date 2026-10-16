#!/bin/sh
# xerj-tray installer.
#
# Builds the tray, installs the binary, the systemd *user* unit that owns the
# XERJ daemon, and the Plasma autostart entry. The unit is deliberately NOT
# enabled: enabling it would start the ~350 MB daemon at every login, which is
# exactly what this tray exists to avoid.
#
# Overridable for testing:
#   BIN_DIR, UNIT_DIR, AUTOSTART_DIR, UNIT_NAME
set -eu

REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIN_DIR=${BIN_DIR:-$HOME/.local/bin}
UNIT_DIR=${UNIT_DIR:-$HOME/.config/systemd/user}
AUTOSTART_DIR=${AUTOSTART_DIR:-$HOME/.config/autostart}
APPS_DIR=${APPS_DIR:-$HOME/.local/share/applications}
UNIT_NAME=${UNIT_NAME:-xerj}

BOLD=''; DIM=''; RED=''; GRN=''; YEL=''; RST=''
if [ -t 1 ]; then
  BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m'); RED=$(printf '\033[31m')
  GRN=$(printf '\033[32m'); YEL=$(printf '\033[33m'); RST=$(printf '\033[0m')
fi

say()  { printf '%s\n' "$*"; }
info() { printf '%s==>%s %s\n' "$YEL" "$RST" "$*"; }
ok()   { printf '%s ok %s %s\n' "$GRN" "$RST" "$*"; }
warn() { printf '%swarn:%s %s\n' "$YEL" "$RST" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$RED" "$RST" "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "'$1' is required but not found in PATH"; }
need cargo; need systemctl; need install; need sed

info "building the tray (release)…"
( cd "$REPO_DIR" && cargo build --release )

info "installing the tray binary…"
install -Dm755 "$REPO_DIR/target/release/xerj-tray" "$BIN_DIR/xerj-tray"
ok "$BIN_DIR/xerj-tray"

info "installing the systemd user unit…"
install -Dm644 "$REPO_DIR/packaging/xerj.service" "$UNIT_DIR/$UNIT_NAME.service"
ok "$UNIT_DIR/$UNIT_NAME.service"

info "installing the desktop entries (launcher + autostart)…"
mkdir -p "$AUTOSTART_DIR" "$APPS_DIR"
sed "s|@HOME@|$HOME|g" "$REPO_DIR/packaging/xerj-tray.desktop.in" > "$AUTOSTART_DIR/xerj-tray.desktop"
chmod 644 "$AUTOSTART_DIR/xerj-tray.desktop"
sed "s|@HOME@|$HOME|g" "$REPO_DIR/packaging/xerj-tray.desktop.in" > "$APPS_DIR/xerj-tray.desktop"
chmod 644 "$APPS_DIR/xerj-tray.desktop"
ok "$AUTOSTART_DIR/xerj-tray.desktop (starts with the session)"
ok "$APPS_DIR/xerj-tray.desktop (appears in the launcher)"
# Refresh the desktop/launcher caches when the tools exist; KDE also watches
# these directories, so this is belt and braces.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true

info "reloading the systemd user manager…"
if systemctl --user daemon-reload; then
  ok "daemon-reload"
else
  warn "could not reload the user manager (no session bus?) — run 'systemctl --user daemon-reload' from your desktop session"
fi

if [ ! -x "$BIN_DIR/xerj" ]; then
  warn "$BIN_DIR/xerj not found — install the engine first: curl -fsSL https://xerj.org/get | sh"
fi

say ""
say "${BOLD}Installed.${RST}"
say "  start the tray:        $BIN_DIR/xerj-tray &"
say "  start the daemon:      systemctl --user start $UNIT_NAME"
say "  daemon status:         systemctl --user status $UNIT_NAME"
say "  ${DIM}the daemon unit is not enabled on purpose (it would use ~350 MB at every login)${RST}"
say "  ${DIM}to keep it always on instead: systemctl --user enable $UNIT_NAME${RST}"
say "  uninstall:             $REPO_DIR/uninstall.sh"
