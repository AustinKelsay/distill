#!/usr/bin/env bash
# Build and install distill, distill-mcp and distill-gui into Cargo's bin directory
# ($CARGO_INSTALL_ROOT, else $CARGO_HOME, else ~/.cargo, plus /bin).
#
# On macOS, with the identity from scripts/setup-signing.sh, each binary is signed with a stable
# identifier and certificate, so Full Disk Access granted to it survives reinstalls. Without
# that identity, the binaries keep cargo's ad-hoc signature, and each reinstall resets the grant.
#
# Everything is built and signed in a staging folder first; the installed binaries are replaced
# only once all three are ready, so a failed build or signature leaves the old ones in place.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME="Distill Local Code Signing"
ROOT="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
BINS=(distill distill-mcp distill-gui)

# Whether a tick is running. `pgrep` exits 1 for no match; anything else is an error, not "no".
ticking() {
  command -v pgrep >/dev/null || return 1
  local rc=0
  # `tick` may follow global options (`distill --home ~/x tick`, `distill --json tick`).
  pgrep -f 'distill( .*)? tick( |$)' >/dev/null || rc=$?
  case $rc in
    0) return 0 ;;
    1) return 1 ;;
    *) echo "Could not check for a running distill tick (pgrep exited $rc)." >&2; exit 1 ;;
  esac
}
if ticking; then
  echo "A distill tick is running; wait for it to finish, then run this again." >&2
  exit 1
fi

mkdir -p "$ROOT/bin"
# On the same filesystem as the destination, so moving a binary into place is one rename.
stage=$(mktemp -d "$ROOT/.distill-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT

for crate in distill-cli distill-mcp distill-gui; do
  cargo install --path "crates/$crate" --locked --quiet --root "$stage" --no-track
done

signed=false
if [[ "$(uname)" == Darwin ]]; then
  ids=$(security find-identity -v -p codesigning | awk -v n="\"$NAME\"" '$0 ~ n {print $2}')
  count=$(printf '%s' "$ids" | grep -c . || true)
  if [[ "$count" -gt 1 ]]; then
    echo "There are $count valid identities named \"$NAME\"; keep one in Keychain Access." >&2
    exit 1
  fi
  if [[ "$count" -eq 1 ]]; then
    for bin in "${BINS[@]}"; do
      codesign --force --sign "$ids" --identifier "dev.distill.$bin" "$stage/bin/$bin"
    done
    signed=true
  fi
fi

if ticking; then
  echo "A distill tick started during the build; run this again when it has finished." >&2
  exit 1
fi
for bin in "${BINS[@]}"; do
  mv -f "$stage/bin/$bin" "$ROOT/bin/$bin"
done

if $signed; then
  echo "Installed into $ROOT/bin, signed with \"$NAME\": privacy grants survive this reinstall."
elif [[ "$(uname)" == Darwin ]]; then
  echo "Installed into $ROOT/bin with ad-hoc signatures. Run scripts/setup-signing.sh once," >&2
  echo "then this again, or Full Disk Access has to be granted again after every install." >&2
fi
