#!/usr/bin/env bash
# Builds a macOS .app bundle around the GPUI binary.
#
# The SwiftUI build ships as a menu-bar app with no Dock icon; this reproduces
# that shape: LSUIElement, an .icns, and the bundle identifier the settings
# domain already uses. Several behaviours only exist when the process really is a
# bundle (AppKit refuses to report a frontmost application otherwise), which is
# why packaging is not just a release step.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${PROFILE:-debug}"
# The shipped build keeps the Swift app's identifier so the settings domain and
# installed app are replaced in place. Use BUNDLE_ID=...gpui for a side-by-side
# test copy.
BUNDLE_ID="${BUNDLE_ID:-com.kaigedong.manispocket}"
APP_NAME="Manis Pocket"
OUT="$REPO/build/$APP_NAME.app"

cd "$REPO"
if [ "$PROFILE" = release ]; then
  cargo build --release -p manis-pocket-app
  BINARY="$REPO/target/release/manis-pocket"
else
  cargo build -p manis-pocket-app
  BINARY="$REPO/target/debug/manis-pocket"
fi

echo "assembling $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BINARY" "$OUT/Contents/MacOS/manis-pocket"

# Icon: grow an .icns from the existing 1024pt asset so Finder/Dock have one.
SOURCE_ICON="$REPO/ManisPocket/Assets.xcassets/AppIcon.appiconset/AppIcon (Big Sur)-1024w.png"
if [ -f "$SOURCE_ICON" ]; then
  ICONSET="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$ICONSET"
  for size in 16 32 64 128 256 512; do
    sips -z $size $size "$SOURCE_ICON" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$SOURCE_ICON" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$OUT/Contents/Resources/AppIcon.icns"
else
  echo "warning: app icon asset missing, bundle will have no icon" >&2
fi

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>manis-pocket</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>2.7.127</string>
  <key>CFBundleVersion</key><string>2.7.127</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <!-- Menu-bar app: no Dock icon, like the SwiftUI build. -->
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: enough for local runs and for AppKit to treat it as a real app.
codesign --force --sign - "$OUT" >/dev/null 2>&1 || echo "warning: ad-hoc signing failed" >&2

echo "built $OUT"
du -sh "$OUT"
