#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
APP_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(cd "$APP_DIR/../.." && pwd)
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
DIST_DIR="$APP_DIR/dist/linux"
PACKAGE_DIR="$DIST_DIR/distill-desktop"
BINARY_NAME="distill-desktop"
ARCH=$(uname -m)
TARBALL="$DIST_DIR/distill-desktop-linux-$ARCH.tar.gz"

cargo build --manifest-path "$APP_DIR/Cargo.toml" --release

rm -rf "$PACKAGE_DIR" "$TARBALL"
mkdir -p "$PACKAGE_DIR"

cp "$TARGET_DIR/release/$BINARY_NAME" "$PACKAGE_DIR/$BINARY_NAME"
chmod +x "$PACKAGE_DIR/$BINARY_NAME"

# NOTE: The .desktop file assumes distill-desktop is in PATH.
# Users should either copy the binary to ~/.local/bin or add the
# extracted directory to their PATH.
cat > "$PACKAGE_DIR/distill-desktop.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Distill Desktop
Exec=distill-desktop
Terminal=false
Categories=Development;
EOF

tar -C "$DIST_DIR" -czf "$TARBALL" "$(basename "$PACKAGE_DIR")"

echo "Staged Linux bundle at $PACKAGE_DIR"
echo "Created tarball at $TARBALL"
