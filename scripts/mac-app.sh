#!/usr/bin/env bash
# Build DISTILL.app around a built window binary (macOS only).
#
#   scripts/mac-app.sh <distill-gui binary> <output folder> <distill command path>
#
# Writes <output folder>/DISTILL.app: the window as Contents/MacOS/DISTILL (so the menu bar,
# About and Quit say DISTILL), the icon made from crates/distill-gui/assets/icon.png, and an
# Info.plist whose LSEnvironment names the distill command, so Finish can install the timer
# when the app is opened from Finder. The bundle is signed ad hoc. install.sh calls this.
set -euo pipefail
cd "$(dirname "$0")/.."

[[ $# -eq 3 ]] || { echo "usage: $0 <distill-gui binary> <output folder> <distill command path>" >&2; exit 2; }
gui=$1 out=$2 cli=$3
[[ "$(uname)" == Darwin ]] || { echo "DISTILL.app is macOS only" >&2; exit 1; }
[[ -x "$gui" ]] || { echo "not an executable: $gui" >&2; exit 1; }
[[ "$cli" == /* ]] || { echo "the distill command path must be absolute: $cli" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app="$out/DISTILL.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$gui" "$app/Contents/MacOS/DISTILL"

# The icon: every size iconutil wants, from the 1024 px source.
iconset=$(mktemp -d)/DISTILL.iconset
mkdir -p "$iconset"
src=crates/distill-gui/assets/icon.png
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$src" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$src" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/DISTILL.icns"
rm -rf "$(dirname "$iconset")"

xml() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' <<<"$1"; }
cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DISTILL</string>
  <key>CFBundleDisplayName</key><string>DISTILL</string>
  <key>CFBundleIdentifier</key><string>dev.distill.app</string>
  <key>CFBundleExecutable</key><string>DISTILL</string>
  <key>CFBundleIconFile</key><string>DISTILL</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$(xml "$version")</string>
  <key>CFBundleVersion</key><string>$(xml "$version")</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSEnvironment</key>
  <dict>
    <key>DISTILL_CLI</key><string>$(xml "$cli")</string>
  </dict>
</dict>
</plist>
EOF
plutil -lint -s "$app/Contents/Info.plist"
codesign --force --sign - "$app"
