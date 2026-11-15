#!/usr/bin/env bash
# Build and install distill, distill-mcp and distill-gui into ~/.cargo/bin.
#
# On macOS, with the identity from scripts/setup-signing.sh, each binary is signed with a stable
# identifier and certificate, so Full Disk Access granted to it survives reinstalls. Without
# that identity, the binaries keep cargo's ad-hoc signature, and each reinstall resets the grant.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME="Distill Local Code Signing"

# A tick running during the swap could see a half-written binary.
if command -v pgrep >/dev/null && pgrep -qf 'distill tick'; then
  echo "A distill tick is running; wait for it to finish, then run this again." >&2
  exit 1
fi

for crate in distill-cli distill-mcp distill-gui; do
  cargo install --path "crates/$crate" --locked --quiet
done

if [[ "$(uname)" == Darwin ]]; then
  if security find-identity -v -p codesigning | grep -q "\"$NAME\""; then
    for bin in distill distill-mcp distill-gui; do
      codesign --force --sign "$NAME" --identifier "dev.distill.$bin" "$HOME/.cargo/bin/$bin"
    done
    echo "Signed with \"$NAME\"; privacy grants survive this reinstall."
  else
    echo "Not signed with a stable identity: run scripts/setup-signing.sh once, then reinstall," >&2
    echo "or Full Disk Access has to be granted again after every install." >&2
  fi
fi
