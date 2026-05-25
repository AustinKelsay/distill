#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
APP_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(cd "$APP_DIR/../.." && pwd)
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
DIST_DIR="$APP_DIR/dist/macos"
APP_NAME="Distill Desktop"
BINARY_NAME="distill-desktop"
APP_BUNDLE="$DIST_DIR/$APP_NAME.app"

cargo build --manifest-path "$APP_DIR/Cargo.toml" --release

rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "$TARGET_DIR/release/$BINARY_NAME" "$APP_BUNDLE/Contents/MacOS/$APP_NAME"
chmod +x "$APP_BUNDLE/Contents/MacOS/$APP_NAME"

cat > "$APP_BUNDLE/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>Distill Desktop</string>
  <key>CFBundleExecutable</key>
  <string>Distill Desktop</string>
  <key>CFBundleIdentifier</key>
  <string>dev.distill.desktop</string>
  <key>CFBundleName</key>
  <string>Distill Desktop</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>
EOF

echo "Staged macOS bundle at $APP_BUNDLE"
