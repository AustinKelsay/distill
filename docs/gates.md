# Distill Verification Gates

Canonical rebuild verification commands for the Rust Library, thin callers, desktop host, and legacy Electron baseline.

Run from the repository root with a modern Node toolchain on `PATH` when desktop or legacy npm suites are included.

## Core Rust gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

## Sync / Source settings gates (#22)

```bash
cargo test -p distill-library --test library_ops_sync --features test-leases
cargo test -p distill-cli --test cli_fixture_journey
cargo test -p distill-desktop --test host_fixture_journey
npm run desktop:test
```

Provider subprocess duration bounds and large-stdin cleanup are covered on macOS/Linux. Output-byte caps are covered on all platforms via a deterministic helper without spawning. Heartbeat accuracy uses the non-default `test-leases` feature only.

## Desktop gates

```bash
npm run desktop:typecheck
npm run desktop:test
npm run desktop:frontend:build
cargo build -p distill-desktop
# release host proof without packaging:
cargo tauri build --no-bundle
# or npm run desktop:build when configured
npm run desktop:package:macos
npm run desktop:smoke:macos
# Ubuntu CI only:
npm run desktop:package:linux
npm run desktop:smoke:linux
npm --prefix apps/distill-desktop run test:a11y
npm --prefix apps/distill-desktop run a11y:smoke
cargo test -p distill-library --test library_scale_budgets
# scheduled/manual full corpus (logical 10 GiB padding; never a default PR gate):
DISTILL_SCALE_BENCH=1 cargo test -p distill-library --test library_scale_budgets -- --ignored --nocapture
```

The a11y smoke is a post-build renderer check. It does not claim signed packaged
WebView or screen-reader coverage; the human checklist at
`apps/distill-desktop/docs/a11y-human-checklist.md` records assistive technology
observations. On macOS, `desktop:package:macos` builds an `.app` through
the workspace-installed Tauri CLI with `--no-sign`, and `desktop:smoke:macos` proves
the local ad-hoc bundle metadata, restricted capability source, Fixture sync,
search/detail/curation/export journey, quit/relaunch, artifact persistence, and
write containment. It does not claim Developer ID signing, hardened runtime,
notarization, migration, crash recovery, privacy, scale, export atomicity, or
VoiceOver coverage. The Cargo `tauri` subcommand is not required for this package
gate and is unavailable in the recorded environment.

Linux packaging is a Linux-only CI gate in `.github/workflows/linux-package-smoke.yml`.
It installs Ubuntu WebKitGTK/GTK, Xvfb, dbus, AT-SPI, and `xdotool` dependencies,
builds `.deb` and AppImage artifacts, verifies the Debian `Depends` metadata, installs
the `.deb`, and drives the installed `/usr/bin/distill-desktop` under Ubuntu's
`dbus-run-session`/Xvfb environment. The smoke verifies the installed control tree and
checks the same
Fixture/search/detail/train-curation/export/restart/artifact/containment path as macOS.
It does not claim screen-reader, migration, crash-recovery, privacy, scale, or export
atomicity coverage.

Scale reports are Library-only JSON evidence. The default test is a bounded synthetic
smoke; the 25k Session / 1M message / 10 GiB logical-home run is environment-gated and
must record hardware, cold/warm samples, p50/p95, progress gaps, and cancel acknowledgement.

## Legacy Electron baseline

```bash
npm test
```

Preferred Node for the documented legacy suite: Node 26 (`/opt/homebrew/Cellar/node/26.0.0/bin` on this machine). Node 22 may hit the known inspector incompatibility.

## Combined launcher

```bash
node scripts/run-library-checks.mjs rebuild   # fmt + clippy + cargo test
node scripts/run-library-checks.mjs library   # same as rebuild
node scripts/run-library-checks.mjs faults    # test-faults suite
node scripts/run-library-checks.mjs desktop   # typecheck/lint/format/test/frontend build
node scripts/run-library-checks.mjs npm       # legacy npm test
node scripts/run-library-checks.mjs all       # full package
```
