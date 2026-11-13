#!/usr/bin/env bash
# Sync the Rust workspace to Omarchy2 and build/run it there.
#
# The GPUI Linux backend needs system libraries and a display server, so the
# Linux build cannot be cross-compiled from macOS. This is the loop we use:
# edit locally, sync the sources, build and optionally launch on the Linux box.
#
# Usage:
#   scripts/linux-dev.sh build          # build only
#   scripts/linux-dev.sh run [seconds]  # build, launch on the desktop, stop
#   scripts/linux-dev.sh check          # cargo check, faster than build
set -euo pipefail

HOST="${MANIS_POCKET_LINUX_HOST:-Omarchy2}"
REMOTE_DIR="${MANIS_POCKET_LINUX_DIR:-manis-pocket-build}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

sync() {
  rsync -az --delete \
    --exclude target --exclude .git --exclude build --exclude .delta \
    --exclude .planning --exclude .omx --exclude android --exclude ManisPocket \
    --exclude '*.log' --exclude docs \
    "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$REPO/crates" "$REPO/vendor" \
    "$HOST:~/$REMOTE_DIR/"
}

remote() {
  # rustup lives in ~/.cargo/bin but is not on the login PATH over SSH.
  ssh "$HOST" "export PATH=\$HOME/.cargo/bin:\$PATH; cd ~/$REMOTE_DIR && $*"
}

case "${1:-build}" in
  build)
    sync
    remote "cargo build -p manis-pocket-app 2>&1 | grep -E '^(error|warning: unused)' -A5 || true; cargo build -p manis-pocket-app 2>&1 | tail -2"
    ;;
  check)
    sync
    remote "cargo check -p manis-pocket-app 2>&1 | tail -5"
    ;;
  run)
    seconds="${2:-8}"
    sync
    remote "cargo build -p manis-pocket-app 2>&1 | tail -1"
    remote "rm -rf /tmp/mp-dev && mkdir -p /tmp/mp-dev && timeout $seconds env WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/\$(id -u) MANIS_POCKET_DB=/tmp/mp-dev/db.sqlite ./target/debug/manis-pocket 2>&1 | head -30"
    ;;
  windows)
    # Lists what Hyprland currently sees, so a launched app can be confirmed.
    remote "sig=\$(tr '\\0' '\\n' < /proc/\$(pgrep -x manis-pocket | head -1)/environ | grep HYPRLAND_INSTANCE_SIGNATURE | cut -d= -f2); env HYPRLAND_INSTANCE_SIGNATURE=\$sig XDG_RUNTIME_DIR=/run/user/\$(id -u) hyprctl clients 2>&1 | grep -E 'title:|class:'"
    ;;
  *)
    echo "usage: $0 {build|check|run [seconds]|windows}" >&2
    exit 2
    ;;
esac
