#!/usr/bin/env bash
# Build and install distill, distill-mcp and the backup adapters (adapters/) into Cargo's bin
# directory ($CARGO_INSTALL_ROOT, else $CARGO_HOME, else ~/.cargo, plus /bin), and the window:
# on macOS as DISTILL.app in ~/Applications (scripts/mac-app.sh builds it; `distill gui` opens
# it), elsewhere as distill-gui beside the others.
#
# `--spoke` installs only `distill`, for a machine that only captures (ADR 0010): no window, no
# MCP server, no backup adapters, and it removes the window and MCP server an earlier install left.
#
# Everything is built in a staging folder first; the installed files are replaced only once all
# of them are ready, so a failed build leaves the old ones in place.
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
BINS=(distill distill-mcp distill-gui)
CRATES=(distill-cli distill-mcp distill-gui)
MAC=$([[ "$(uname)" == Darwin ]] && echo 1 || true)
SPOKE=
case "${1:-}" in
  "") ;;
  --spoke) SPOKE=1 BINS=(distill) CRATES=(distill-cli) ;;
  *) echo "usage: $0 [--spoke]" >&2; exit 2 ;;
esac
APPS="$HOME/Applications"

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

for crate in "${CRATES[@]}"; do
  cargo install --path "crates/$crate" --locked --quiet --root "$stage" --no-track
done
if [[ -n "$MAC" && -z "$SPOKE" ]]; then
  BINS=(distill distill-mcp)
  scripts/mac-app.sh "$stage/bin/distill-gui" "$stage" "$ROOT/bin/distill"
fi

if ticking; then
  echo "A distill tick started during the build; run this again when it has finished." >&2
  exit 1
fi
for bin in "${BINS[@]}"; do
  mv -f "$stage/bin/$bin" "$ROOT/bin/$bin"
done
if [[ -z "$SPOKE" ]]; then
  for adapter in adapters/distill-backup-*; do
    install -m 755 "$adapter" "$stage/${adapter##*/}"
    mv -f "$stage/${adapter##*/}" "$ROOT/bin/${adapter##*/}"
  done
else
  # A spoke serves no reader: an earlier full install's MCP server and window would read it.
  rm -f "$ROOT/bin/distill-mcp" "$ROOT/bin/distill-gui"
  if [[ -n "$MAC" && -e "$APPS/DISTILL.app" ]]; then mv "$APPS/DISTILL.app" "$stage/previous.app"; fi
fi
if [[ -n "$MAC" ]]; then
  if [[ -z "$SPOKE" ]]; then
    # One copy of the window: the app. `distill gui` opens it.
    rm -f "$ROOT/bin/distill-gui"
    mkdir -p "$APPS"
    # The old app moves into the staging folder (removed on exit); an open window keeps running.
    if [[ -e "$APPS/DISTILL.app" ]]; then mv "$APPS/DISTILL.app" "$stage/previous.app"; fi
    mv "$stage/DISTILL.app" "$APPS/DISTILL.app"
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
      -f "$APPS/DISTILL.app" || true
  fi
  # The timer's job is pinned to the binary it last launched, and a new build fails that pin
  # once (#175). Loading the job again clears it; RunAtLoad then runs the catch-up tick.
  plist="$HOME/Library/LaunchAgents/dev.distill.tick.plist"
  job="gui/$(id -u)/dev.distill.tick"
  if [[ -f "$plist" ]]; then
    # Best effort: a tick the timer starts between this check and `bootout` is stopped, and the
    # catch-up tick goes on from it.
    if ticking; then
      echo "A distill tick started; the timer was left as it was, and may skip its next run." >&2
    else
      launchctl bootout "$job" 2>/dev/null || true
      if ! launchctl bootstrap "gui/$(id -u)" "$plist"; then
        echo "The timer could not be loaded again; load it with: launchctl bootstrap gui/$(id -u) $(printf '%q' "$plist")" >&2
        exit 1
      fi
      echo "Reloaded the timer; it runs a tick now."
    fi
  fi
  if [[ -z "$SPOKE" ]]; then
    echo "Installed into $ROOT/bin and $APPS/DISTILL.app."
  else
    echo "Installed into $ROOT/bin."
  fi
else
  echo "Installed into $ROOT/bin."
fi
