#!/bin/zsh
# Build Claude Traffic Light and install it as a clickable macOS app.
# Usage: scripts/make-app.sh [install-dir]   (default: ~/Applications)
set -euo pipefail

ROOT="${0:A:h:h}"
DEST_DIR="${1:-$HOME/Applications}"
APP="$DEST_DIR/Claude Traffic Light.app"

cd "$ROOT"
cargo build --release -p ctl-app --features tauri/custom-protocol

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Build an .icns from the 128px PNG.
ICONSET="$WORK/icon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 64 128; do
  sips -z $size $size src-tauri/icons/icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
done
cp "$ICONSET/icon_32x32.png" "$ICONSET/icon_16x16@2x.png"
cp "$ICONSET/icon_64x64.png" "$ICONSET/icon_32x32@2x.png"
rm "$ICONSET/icon_64x64.png"
iconutil -c icns "$ICONSET" -o "$WORK/AppIcon.icns"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/ctl-app "$APP/Contents/MacOS/ctl-app"
cp "$WORK/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Claude Traffic Light</string>
  <key>CFBundleDisplayName</key><string>Claude Traffic Light</string>
  <key>CFBundleIdentifier</key><string>com.claudetrafficlight.app</string>
  <key>CFBundleExecutable</key><string>ctl-app</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "Installed: $APP"
