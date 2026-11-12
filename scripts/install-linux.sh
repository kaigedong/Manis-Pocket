#!/usr/bin/env bash
# Installs the GPUI build for the current user on Linux.
#
# No root: the binary goes to ~/.local/bin, the launcher entry to
# ~/.local/share/applications, and an autostart entry makes it resident from
# login. `--uninstall` reverses all three.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${PROFILE:-release}"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
AUTOSTART_DIR="$HOME/.config/autostart"

if [ "${1:-install}" = "--uninstall" ]; then
  rm -f "$BIN_DIR/manis-pocket" "$APP_DIR/manis-pocket.desktop" "$AUTOSTART_DIR/manis-pocket.desktop"
  echo "uninstalled"
  exit 0
fi

# rustup installs into ~/.cargo/bin, which is not on the PATH of every shell.
if [ -d "$HOME/.cargo/bin" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

cd "$REPO"
if [ "$PROFILE" = release ]; then
  cargo build --release -p manis-pocket-app
  BINARY="$REPO/target/release/manis-pocket"
else
  cargo build -p manis-pocket-app
  BINARY="$REPO/target/debug/manis-pocket"
fi

mkdir -p "$BIN_DIR" "$APP_DIR" "$AUTOSTART_DIR"
install -m 755 "$BINARY" "$BIN_DIR/manis-pocket"

cat > "$APP_DIR/manis-pocket.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Manis Pocket
Comment=Clipboard history with device sync
Exec=$BIN_DIR/manis-pocket
Terminal=false
Categories=Utility;
StartupNotify=false
# Matches the window app id, so the compositor ties the window to this entry.
StartupWMClass=io.github.kaigedong.ManisPocket
DESKTOP

# Start with the session: the panel is only the UI, the process is the watcher.
cp "$APP_DIR/manis-pocket.desktop" "$AUTOSTART_DIR/manis-pocket.desktop"
sed -i 's/^Terminal=false/Terminal=false\nX-GNOME-Autostart-enabled=true/' "$AUTOSTART_DIR/manis-pocket.desktop"

echo "installed to $BIN_DIR/manis-pocket"
command -v desktop-file-validate >/dev/null && desktop-file-validate "$APP_DIR/manis-pocket.desktop" && echo "desktop entry valid"
