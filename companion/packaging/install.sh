#!/usr/bin/env bash
#
# Install the Notion GNOME companion ecosystem for the current user:
#   * builds notion-watcher (Component A) and notion-quickview (Component C)
#   * installs both binaries to ~/.local/bin
#   * installs + enables the notion-watcher systemd *user* service
#   * installs the GNOME Shell extension (Component B) and compiles its schema
#
# Everything is per-user (no root). Re-run any time to update.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPANION="$REPO_ROOT/companion"
BIN_DIR="$HOME/.local/bin"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
DBUS_SERVICES_DIR="$HOME/.local/share/dbus-1/services"
EXT_UUID="notion-island@notion.app"
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/$EXT_UUID"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m warning:\033[0m %s\n' "$*"; }

command -v cargo >/dev/null || { echo "cargo (Rust) is required"; exit 1; }

info "Building notion-watcher (daemon)…"
cargo build --release --manifest-path "$COMPANION/notion-watcher/Cargo.toml"

info "Building notion-quickview (GTK app)…"
if ! cargo build --release --manifest-path "$COMPANION/notion-quickview/Cargo.toml"; then
    warn "notion-quickview failed to build — install libgtk-4-dev libadwaita-1-dev libdbus-1-dev and re-run."
    warn "The daemon + extension will still be installed."
fi

info "Installing binaries to $BIN_DIR…"
mkdir -p "$BIN_DIR"
install -m 0755 "$COMPANION/notion-watcher/target/release/notion-watcher" "$BIN_DIR/notion-watcher"
if [[ -f "$COMPANION/notion-quickview/target/release/notion-quickview" ]]; then
    install -m 0755 "$COMPANION/notion-quickview/target/release/notion-quickview" "$BIN_DIR/notion-quickview"
fi

info "Installing systemd user service…"
mkdir -p "$SYSTEMD_USER_DIR"
sed "s#^ExecStart=.*#ExecStart=$BIN_DIR/notion-watcher#" \
    "$COMPANION/notion-watcher/data/notion-watcher.service" \
    > "$SYSTEMD_USER_DIR/notion-watcher.service"

info "Installing DBus activation file…"
mkdir -p "$DBUS_SERVICES_DIR"
sed "s#^Exec=.*#Exec=$BIN_DIR/notion-watcher#" \
    "$COMPANION/notion-watcher/data/com.notion.Calendar.service" \
    > "$DBUS_SERVICES_DIR/com.notion.Calendar.service"

info "Installing GNOME Shell extension…"
mkdir -p "$EXT_DIR"
cp -r "$COMPANION/gnome-extension/$EXT_UUID/." "$EXT_DIR/"
if command -v glib-compile-schemas >/dev/null; then
    glib-compile-schemas "$EXT_DIR/schemas/"
else
    warn "glib-compile-schemas not found; extension settings may not load."
fi

info "Enabling + starting the watcher service…"
if command -v systemctl >/dev/null; then
    systemctl --user daemon-reload
    systemctl --user enable --now notion-watcher.service || \
        warn "Could not start the service now (no graphical session?). It will start at next login."
else
    warn "systemctl not found; start the daemon manually: $BIN_DIR/notion-watcher"
fi

info "Enabling the GNOME extension…"
if command -v gnome-extensions >/dev/null; then
    gnome-extensions enable "$EXT_UUID" || \
        warn "Run 'gnome-extensions enable $EXT_UUID' after logging back in."
else
    warn "gnome-extensions CLI not found; enable 'Notion Dynamic Island' via the Extensions app."
fi

cat <<EOF

Done. Notes:
  • Ensure \$HOME/.local/bin is on your PATH.
  • Log out/in (or on Xorg press Alt+F2, r) so GNOME Shell loads the extension.
  • Unlock the Notion desktop app once so it publishes the DB key to your keyring;
    the daemon and quick-view read it from there.
EOF
