#!/usr/bin/env bash
#
# Remove the Notion GNOME companion ecosystem for the current user.
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
DBUS_SERVICES_DIR="$HOME/.local/share/dbus-1/services"
EXT_UUID="notion-island@notion.app"
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/$EXT_UUID"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

info "Disabling GNOME extension…"
command -v gnome-extensions >/dev/null && gnome-extensions disable "$EXT_UUID" 2>/dev/null || true
rm -rf "$EXT_DIR"

info "Stopping + removing the watcher service…"
if command -v systemctl >/dev/null; then
    systemctl --user disable --now notion-watcher.service 2>/dev/null || true
    systemctl --user daemon-reload 2>/dev/null || true
fi
rm -f "$SYSTEMD_USER_DIR/notion-watcher.service"
rm -f "$DBUS_SERVICES_DIR/com.notion.Calendar.service"

info "Removing binaries…"
rm -f "$BIN_DIR/notion-watcher" "$BIN_DIR/notion-quickview"

info "Done. Your calendar data in the shared database is untouched."
