#!/bin/sh
# Undo install.sh: remove the tray binary, the systemd user unit and the
# Plasma autostart entry. Leaves the XERJ binary and its data directory alone.
set -eu

BIN_DIR=${BIN_DIR:-$HOME/.local/bin}
UNIT_DIR=${UNIT_DIR:-$HOME/.config/systemd/user}
AUTOSTART_DIR=${AUTOSTART_DIR:-$HOME/.config/autostart}
APPS_DIR=${APPS_DIR:-$HOME/.local/share/applications}
UNIT_NAME=${UNIT_NAME:-xerj}

systemctl --user stop "$UNIT_NAME" 2>/dev/null || true
rm -f "$UNIT_DIR/$UNIT_NAME.service" \
      "$AUTOSTART_DIR/xerj-tray.desktop" \
      "$APPS_DIR/xerj-tray.desktop" \
      "$BIN_DIR/xerj-tray"
systemctl --user daemon-reload 2>/dev/null || true
printf 'removed: %s %s %s %s\n' \
  "$BIN_DIR/xerj-tray" \
  "$UNIT_DIR/$UNIT_NAME.service" \
  "$AUTOSTART_DIR/xerj-tray.desktop" \
  "$APPS_DIR/xerj-tray.desktop"
